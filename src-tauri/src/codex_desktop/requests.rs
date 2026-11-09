//! Route live native requests through the existing first-answer-wins coordinator.
use super::{Bridge, Session};
use crate::{
    ipc::{self, ClientMsg, ServerMsg},
    models::*,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

#[derive(Default)]
pub struct Tracker {
    seen: HashSet<String>,
    tasks: HashMap<String, (String, tokio::task::JoinHandle<()>)>,
}
impl Tracker {
    pub fn tick(&mut self, bridge: &Arc<Bridge>, waiting: &[(String, String, String)]) {
        let sessions = bridge.sessions.lock().unwrap().clone();
        let mut live = HashSet::new();
        let permission_enabled = crate::integrations::agent_permission::enabled(
            crate::integrations::agent_rules::AgentTarget::Codex,
        );
        // A custom Codex home has its own hook installation; the permission preference is shared.
        let hook_owns_approval = super::runtime::detect(&bridge.config.lock().unwrap())
            .ok()
            .and_then(|install| std::fs::read_to_string(install.home.join("hooks.json")).ok())
            .is_some_and(|text| text.contains(crate::integrations::agent_permission::MARKER));
        for (sid, s) in sessions.iter().filter(|(_, s)| s.connected) {
            for request in &live_requests(s) {
                let key = owned_key(sid, s, request);
                live.insert(key.clone());
                if self.seen.contains(&key) {
                    continue;
                }
                let approval = request["method"]
                    .as_str()
                    .is_some_and(|m| m.ends_with("requestApproval"));
                // Installed hooks own approvals, including remembered decisions. Never create a
                // second decision surface for the same native operation.
                if approval && (!permission_enabled || hook_owns_approval) {
                    self.seen.insert(key);
                    continue;
                }
                if waiting.iter().any(|(id, _, _)| id == sid)
                    && !crate::client::recovery::owned_exists(&key).unwrap_or(false)
                {
                    continue;
                }
                let Some(message) = prepare(sid, s, request) else {
                    self.seen.insert(key);
                    continue;
                };
                self.seen.insert(key.clone());
                let b = bridge.clone();
                let id = sid.clone();
                let s = s.clone();
                let request = request.clone();
                let task = tokio::spawn(async move {
                    if let Err(error) = forward(&b, &id, &s, &request, message).await {
                        if let Some(session) = b.sessions.lock().unwrap().get_mut(&id) {
                            session.error = Some(format!("Answer in Codex App: {error}"));
                        }
                    }
                });
                self.tasks.insert(key, (sid.clone(), task));
            }
        }
        if let Err(error) = reconcile_consumers(&sessions, &live) {
            eprintln!("askhuman: waiting to reconcile native questions: {error}");
        }
        self.tasks.retain(|key, (sid, task)| {
            let authoritative = sessions.get(sid).is_some_and(|session| session.connected);
            if authoritative && !live.contains(key) {
                task.abort();
                return false;
            }
            !task.is_finished()
        });
        self.seen
            .retain(|key| live.contains(key) || self.tasks.contains_key(key));
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        for (_, task) in self.tasks.values() {
            task.abort();
        }
    }
}

fn owned_key(sid: &str, session: &Session, request: &Value) -> String {
    use sha2::Digest;
    let identity = json!([
        sid,
        session.owner,
        request["id"],
        request["method"],
        request["params"]
    ]);
    format!(
        "codex/{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&identity).unwrap())
    )
}

fn prepare(sid: &str, s: &Session, r: &Value) -> Option<ClientMsg> {
    let method = r["method"].as_str()?;
    let project = s.meta.as_ref()?.cwd.clone();
    let lang = crate::i18n::Lang::current();
    let zh = lang == crate::i18n::Lang::Zh;
    match method {
        "item/tool/requestUserInput"
        | "tool/requestUserInput"
        | "askhuman/requestUserInputAsync" => {
            let raw = r["params"]["questions"].as_array()?;
            if raw.is_empty() || raw.len() > 8 || raw.iter().any(|q| q["isSecret"] == true) {
                return None;
            }
            let questions = raw
                .iter()
                .map(|q| {
                    q["id"].as_str()?;
                    let text = q["question"].as_str()?;
                    let options = q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| OptionItem::new(option_text(o), false))
                        .collect();
                    Some(Question::new(text.into(), options))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(ClientMsg::Submit(ipc::TaskRequest {
                caller_cwd: None,
                native_request_id: Some(format!("{:?}:{}", s.owner, r["id"])),
                message: MessagePrompt::default(),
                questions,
                is_markdown: true,
                source: "Codex App".into(),
                lang: lang.code().into(),
                project,
                select_only: false,
                single: true,
                output_format: OutputFormat::Json,
                record_history: true,
                stop_confirmation: false,
                agent_kind: Some("codex".into()),
                agent_session_id: Some(sid.into()),
                mcp_instance_id: None,
                agent_pid: None,
                caller_pid: 0,
                from_mcp: false,
                perf_id: String::new(),
                perf_autodismiss: false,
                whats_next: false,
                escalate: false,
                escalation_reason: None,
            }))
        }
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval" => {
            let available = r["params"]["availableDecisions"].as_array();
            let choices = [
                (
                    "accept",
                    if zh {
                        "仅批准本次"
                    } else {
                        "Approve once"
                    },
                ),
                ("decline", if zh { "拒绝" } else { "Decline" }),
            ]
            .into_iter()
            .filter(|(id, _)| available.is_none_or(|a| a.iter().any(|v| v.as_str() == Some(id))))
            .map(|(id, label)| ConfirmChoice {
                id: id.into(),
                label: label.into(),
                description: String::new(),
                role: crate::confirm::ActionRole::Default,
                variant: None,
            })
            .collect::<Vec<_>>();
            if choices.len() != 2 {
                return None;
            }
            Some(ClientMsg::SubmitConfirm(Box::new(ipc::ConfirmTask {
                development: None,
                caller_cwd: None,
                spec: ConfirmSpec {
                    title: if zh {
                        "Codex App 请求批准"
                    } else {
                        "Codex App requests approval"
                    }
                    .into(),
                    context: vec![],
                    detail: ConfirmDetail {
                        summary: r["params"]["reason"].as_str().unwrap_or(method).into(),
                        body_md: format!(
                            "```json\n{}\n```",
                            serde_json::to_string_pretty(&r["params"]).ok()?
                        ),
                    },
                    choices,
                    presentation: ConfirmPresentation::SingleSelectSubmit {
                        input: None,
                        submit_label: if zh { "提交" } else { "Submit" }.into(),
                        default_action_id: None,
                    },
                    dismiss_action_id: "decline".into(),
                },
                popup_edit: None,
                source: "Codex App".into(),
                lang: lang.code().into(),
                project,
                agent_kind: "codex".into(),
                agent_session_id: sid.into(),
                caller_pid: 0,
                memory: None,
            })))
        }
        _ => None,
    }
}
fn option_text(o: &Value) -> String {
    let label = o["label"].as_str().unwrap_or_default();
    match o["description"].as_str().filter(|d| !d.is_empty()) {
        Some(d) => format!("{label} — {d}"),
        None => label.into(),
    }
}
fn still_pending(bridge: &Bridge, sid: &str, owner: &Option<String>, r: &Value) -> Option<bool> {
    let session = bridge.session(sid).filter(|session| session.connected)?;
    Some(
        &session.owner == owner
            && live_requests(&session)
                .iter()
                .any(|current| owned_key(sid, &session, current) == owned_key(sid, &session, r)),
    )
}

fn reconcile_consumers(
    sessions: &HashMap<String, Session>,
    live: &HashSet<String>,
) -> anyhow::Result<()> {
    let path = crate::paths::config_dir().join("state/requests.sqlite3");
    if !path.exists() || !sessions.values().any(|session| session.connected) {
        return Ok(());
    }
    let store = crate::daemon::recovery::Store::open(&path)?;
    for (key, record) in store.owned_records("codex/")? {
        if record.effect != crate::daemon::recovery::OwnedEffect::Waiting || live.contains(&key) {
            continue;
        }
        let sid = match record.task.as_ref() {
            Some(crate::daemon::recovery::Task::Ask(task)) => task.agent_session_id.as_deref(),
            Some(crate::daemon::recovery::Task::Confirm(task)) => {
                Some(task.agent_session_id.as_str())
            }
            None => None,
        };
        if sid
            .and_then(|sid| sessions.get(sid))
            .is_some_and(|session| session.connected)
        {
            store.cancel_owned(&key)?;
        }
    }
    Ok(())
}

/// Async questions are part of the live desktop snapshot, never read from rollout history.
fn live_requests(s: &Session) -> Vec<Value> {
    if !s.connected {
        return vec![];
    }
    let mut requests = s.state["requests"].as_array().cloned().unwrap_or_default();
    let turns = super::protocol::turns(&s.state);
    let Some(turn) = turns
        .last()
        .filter(|t| matches!(t["status"].as_str(), Some("inProgress" | "completed")))
    else {
        return requests;
    };
    let items = super::protocol::entities(&turn["items"]);
    let mut answered = HashSet::new();
    for item in &items {
        if item["type"] == "userMessage"
            || (item["type"] == "steeringUserMessage" && item["status"] == "accepted")
        {
            for part in item
                .get("content")
                .or_else(|| item.get("input"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let text = part["text"].as_str().unwrap_or_default();
                if let Some(body) = text
                    .split("<send_user_message_question_reply>")
                    .nth(1)
                    .and_then(|s| s.split("</send_user_message_question_reply>").next())
                {
                    if let Ok(replies) = serde_json::from_str::<Vec<Value>>(body) {
                        for reply in replies {
                            if let Some(id) = reply["questionItemId"].as_str() {
                                answered.insert(id.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    for item in items {
        if item["type"] != "agentMessage" {
            continue;
        }
        let questions=item["questions"].as_array().into_iter().flatten().enumerate().filter_map(|(i,q)|{
            let id=json!(["request_user_input_async",item["id"],i]).to_string();
            if answered.contains(&id){return None;}
            Some(json!({"id":id,"question":q["title"],"options":q["options"].as_array().into_iter().flatten().map(|o|json!({"label":o})).collect::<Vec<_>>()}))
        }).collect::<Vec<_>>();
        if !questions.is_empty() {
            requests.push(json!({"id":format!("async:{}:{}",turn["turnId"].as_str().unwrap_or_default(),item["id"].as_str().unwrap_or_default()),"method":"askhuman/requestUserInputAsync","params":{"questions":questions}}));
        }
    }
    requests
}

async fn forward(
    bridge: &Arc<Bridge>,
    sid: &str,
    s: &Session,
    r: &Value,
    message: ClientMsg,
) -> Result<(), String> {
    let key = owned_key(sid, s, r);
    let (decision, continuation) = if crate::client::recovery::available().await {
        let task = match &message {
            ClientMsg::Submit(task) => crate::daemon::recovery::Task::Ask(Box::new(task.clone())),
            ClientMsg::SubmitConfirm(task) => crate::daemon::recovery::Task::Confirm(task.clone()),
            _ => return Ok(()),
        };
        let receive = crate::client::recovery::run_owned(key.clone(), task);
        tokio::pin!(receive);
        let (outcome, handle) = loop {
            tokio::select! {
                result = &mut receive => break result,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if still_pending(bridge, sid, &s.owner, r) == Some(false) { let _ = crate::client::recovery::cancel_owned(&key); return Ok(()); }
                }
            }
        };
        if handle.effect != crate::daemon::recovery::OwnedEffect::Waiting {
            return Ok(());
        }
        let decision = match outcome {
            crate::daemon::recovery::Outcome::Confirm(result) => {
                json!({"decision":result.action_id})
            }
            crate::daemon::recovery::Outcome::Ask(result) if result.exit_code == 0 => {
                match serde_json::from_str(&result.stdout) {
                    Ok(decision) => decision,
                    Err(error) => {
                        handle.complete(true).await;
                        return Err(format!("Invalid question answer: {error}"));
                    }
                }
            }
            _ => {
                handle.complete(true).await;
                return Ok(());
            }
        };
        (decision, Some(handle))
    } else {
        let stream = crate::client::connect_stream()
            .await
            .map_err(|e| e.to_string())?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        ipc::write_msg(&mut writer, &message)
            .await
            .map_err(|e| e.to_string())?;
        let receive = async {
            loop {
                match ipc::read_msg::<_, ServerMsg>(&mut reader)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    Some(ServerMsg::ConfirmFinal { result }) => {
                        return Ok(Some(json!({"decision":result.action_id})))
                    }
                    Some(ServerMsg::Final {
                        stdout,
                        exit_code: 0,
                    }) => {
                        return serde_json::from_str(&stdout)
                            .map(Some)
                            .map_err(|e| format!("Invalid question answer: {e}"))
                    }
                    Some(
                        ServerMsg::Accepted { .. }
                        | ServerMsg::ConfirmAccepted { .. }
                        | ServerMsg::Warn { .. },
                    ) => {}
                    _ => return Ok(None),
                }
            }
        };
        tokio::pin!(receive);
        let decision = loop {
            tokio::select! {
                result=&mut receive=>match result? {Some(answer)=>break answer,None=>return Ok(())},
                _=tokio::time::sleep(Duration::from_millis(500))=>if still_pending(bridge,sid,&s.owner,r) == Some(false) {return Ok(());}
            }
        };
        (decision, None)
    };
    while still_pending(bridge, sid, &s.owner, r).is_none() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if still_pending(bridge, sid, &s.owner, r) == Some(false) {
        if let Some(handle) = &continuation {
            handle.complete(true).await;
        }
        return Ok(());
    }
    let Some((method, payload)) = response(r, &decision) else {
        if let Some(handle) = &continuation {
            handle.complete(true).await;
        }
        return Ok(());
    };
    if let Some(handle) = &continuation {
        if !handle.claim().await {
            return Ok(());
        }
    }
    // Sending once is deliberate: native transport failures have unknown outcomes.
    let sent = if r["method"] == "askhuman/requestUserInputAsync" {
        let replies=r["params"]["questions"].as_array().unwrap().iter().map(|q|json!({"questionItemId":q["id"],"question":q["question"],"answer":payload["response"]["answers"][q["id"].as_str().unwrap()]["answers"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")})).collect::<Vec<_>>();
        bridge
            .execute(super::Operation::Send {
                session_id: sid.into(),
                text: format!(
                    "<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",
                    json!(replies)
                ),
                files: vec![],
                id: continuation
                    .as_ref()
                    .map(|handle| handle.call.id.clone())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            })
            .await
    } else {
        bridge.call(sid, method, payload).await
    };
    if let Some(handle) = &continuation {
        handle.complete(sent.is_ok()).await;
    }
    sent.map(|_| ())
}
fn response(r: &Value, answer: &Value) -> Option<(&'static str, Value)> {
    let mut payload = json!({"requestId":r["id"]});
    let method = match r["method"].as_str()? {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = answer["decision"].as_str()?;
            if !matches!(decision, "accept" | "decline") {
                return None;
            }
            payload["decision"] = json!(decision);
            if r["method"] == "item/commandExecution/requestApproval" {
                "thread-follower-command-approval-decision"
            } else {
                "thread-follower-file-approval-decision"
            }
        }
        "item/permissions/requestApproval" => {
            let accept = match answer["decision"].as_str()? {
                "accept" => true,
                "decline" => false,
                _ => return None,
            };
            payload["response"] = json!({"scope":"turn","permissions":if accept {r["params"]["permissions"].clone()}else{json!({})}});
            "thread-follower-permissions-request-approval-response"
        }
        "item/tool/requestUserInput"
        | "tool/requestUserInput"
        | "askhuman/requestUserInputAsync" => {
            if answer["action"] != "answer" {
                return None;
            }
            let mut answers = serde_json::Map::new();
            for (index, q) in r["params"]["questions"].as_array()?.iter().enumerate() {
                let a = answer["answers"]
                    .as_array()?
                    .iter()
                    .find(|a| a["question_index"].as_u64() == Some(index as u64))?;
                let mut values = Vec::new();
                for text in a["selected_options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    let label = q["options"]
                        .as_array()?
                        .iter()
                        .find(|o| option_text(o) == text)?["label"]
                        .as_str()?;
                    values.push(label.to_string());
                }
                if let Some(t) = a["user_input"].as_str().filter(|t| !t.trim().is_empty()) {
                    values.push(t.into());
                }
                if let Some(files) = a["files"].as_array().filter(|a| !a.is_empty()) {
                    values.push(format!("Attached files: {files:?}"));
                }
                if values.is_empty() {
                    return None;
                }
                answers.insert(q["id"].as_str()?.into(), json!({"answers":values}));
            }
            payload["response"] = json!({"answers":answers});
            "thread-follower-submit-user-input"
        }
        _ => return None,
    };
    Some((method, payload))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn async_questions_expire_when_answered_or_disconnected() {
        let mut s = Session {
            connected: true,
            state: Arc::new(
                json!({"turns":[{"turnId":"t","status":"inProgress","items":[{"id":"q","type":"agentMessage","questions":[{"title":"Choose","options":["A"]}]}]}]}),
            ),
            ..Default::default()
        };
        let request = live_requests(&s).pop().unwrap();
        assert_eq!(request["method"], "askhuman/requestUserInputAsync");
        let reply =
            json!([{"questionItemId":request["params"]["questions"][0]["id"],"answer":"A"}]);
        Arc::make_mut(&mut s.state)["turns"][0]["items"].as_array_mut().unwrap().push(json!({"type":"steeringUserMessage","status":"accepted","input":[{"type":"text","text":format!("<send_user_message_question_reply>{reply}</send_user_message_question_reply>")}]}));
        assert!(live_requests(&s).is_empty());
        Arc::make_mut(&mut s.state)["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .pop();
        s.connected = false;
        assert!(live_requests(&s).is_empty());
    }
    #[test]
    fn answers_keep_native_ids_and_strip_display_descriptions() {
        let r = json!({"id":42,"method":"tool/requestUserInput","params":{"questions":[{"id":"stable","question":"Which?","options":[{"label":"A","description":"detail"}]}]}});
        let (_,p)=response(&r,&json!({"action":"answer","answers":[{"question_index":0,"selected_options":["A — detail"]}]})).unwrap();
        assert_eq!(
            p,
            json!({"requestId":42,"response":{"answers":{"stable":{"answers":["A"]}}}})
        );
        assert!(response(&r, &json!({"action":"cancel"})).is_none());
        assert!(response(&r, &json!({"action":"answer","answers":[]})).is_none());
    }
    #[test]
    fn permission_scope_never_expands() {
        let r = json!({"id":"x","method":"item/permissions/requestApproval","params":{"permissions":{"network":{"enabled":true}}}});
        assert_eq!(
            response(&r, &json!({"decision":"accept"})).unwrap().1["response"]["scope"],
            "turn"
        );
        assert_eq!(
            response(&r, &json!({"decision":"decline"})).unwrap().1["response"]["permissions"],
            json!({})
        );
        assert!(response(&r, &json!({"decision":"acceptForSession"})).is_none());
    }
}
