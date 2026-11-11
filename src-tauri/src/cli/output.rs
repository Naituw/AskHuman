//! 结果区块格式化（区块结构固定，字段标记恒英文、不本地化）。
//!
//! 字段标记（`[selected_options]` 等）为固定英文常量（D6），与 `--agent-help`/`--scripting-help`
//! 共用同一常量，保证「AI 看到的实际输出」与 help 文档一致；仅**值文案**（取消/未作答/确认继续）
//! 仍随界面语言本地化。结构（分组顺序/`# Qn`/`---`）不随语言变化。

use crate::annotations::{self, UserAnnotation};
use crate::i18n::{tr, Lang};
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use serde::Serialize;

/// 字段标记（恒英文，不本地化）。图片与文件已合并为单一 `[files]`（D6b）。
pub const MARKER_SELECTED_OPTIONS: &str = "[selected_options]";
pub const MARKER_USER_INPUT: &str = "[user_input]";
pub const MARKER_FILES: &str = "[files]";
pub const MARKER_STATUS: &str = "[status]";
/// The user's notes on passages of the message (spec `message-annotations`): a `> quote` line and
/// the note for each, a blank line between notes.
pub const MARKER_USER_ANNOTATIONS: &str = "[user_annotations]";

/// 剥掉待办选项的展示前缀「执行待办：」（`whatsNext.todoPrefix`），还原待办原文。
/// 构建端与渲染端可能语言不一致（理论上同进程同语言，但求稳）：两种语言的前缀都尝试。
pub fn strip_todo_prefix(text: &str) -> &str {
    for lang in [Lang::En, Lang::Zh] {
        let prefix = tr(lang, "whatsNext.todoPrefix");
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest;
        }
    }
    text
}

/// 取消路径输出。
pub fn cancel_output(lang: Lang) -> String {
    format!("{}\n{}", MARKER_STATUS, tr(lang, "status.cancel"))
}

/// Text output of an automatic reply (away mode / mute): a status block like a cancellation, so
/// the Agent reads it as "no answer from the user, and this is what to do now".
pub fn auto_reply_output(text: &str) -> String {
    format!("{}\n{}", MARKER_STATUS, text)
}

/// JSON output of an automatic reply. `action` is `auto_reply` (never `answer` or `cancel`) and
/// `channel` names which automatic reply it was (`away` / `mute`).
pub fn auto_reply_json(channel: &str, text: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "action": "auto_reply",
        "channel": channel,
        "status": text,
    }))
    .unwrap_or_default()
}

/// Appends a status text to an already rendered answer: the guidance that comes with a one-shot
/// directive sent along with the answer. Text output gets a trailing `[status]` block; JSON output
/// gets its `status` field filled in (an existing status stays, the new text follows it) and, when
/// `directive` names the kind, a `directive` field so a reader can tell the status is guidance
/// that came with a real answer.
pub fn append_status(stdout: &str, text: &str, json: bool, directive: Option<&str>) -> String {
    if json {
        return match serde_json::from_str::<serde_json::Value>(stdout) {
            Ok(serde_json::Value::Object(mut map)) => {
                let merged = match map.get("status").and_then(serde_json::Value::as_str) {
                    Some(existing) if !existing.trim().is_empty() => format!("{existing}\n{text}"),
                    _ => text.to_string(),
                };
                map.insert("status".into(), serde_json::Value::String(merged));
                if let Some(kind) = directive {
                    map.insert("directive".into(), serde_json::Value::String(kind.into()));
                }
                serde_json::to_string_pretty(&serde_json::Value::Object(map))
                    .unwrap_or_else(|_| stdout.to_string())
            }
            // Not an object or not JSON at all: never produce broken JSON.
            _ => stdout.to_string(),
        };
    }
    let body = stdout.trim_end();
    if body.is_empty() {
        auto_reply_output(text)
    } else {
        format!("{body}\n\n{}\n{}", MARKER_STATUS, text)
    }
}

/// 单题的已渲染回答（图片已落盘为路径，文件为透传的绝对路径）。
pub struct RenderedAnswer<'a> {
    pub selected_options: &'a [String],
    pub user_input: Option<&'a str>,
    pub image_paths: &'a [String],
    pub file_paths: &'a [String],
}

impl RenderedAnswer<'_> {
    /// 空回答：没选项、没（去空白后的）输入、没图片、没回复文件。
    fn is_empty(&self) -> bool {
        self.selected_options.is_empty()
            && self.user_input.map(|s| s.trim().is_empty()).unwrap_or(true)
            && self.image_paths.is_empty()
            && self.file_paths.is_empty()
    }

    fn body(&self, lang: Lang) -> String {
        send_output(
            lang,
            self.selected_options,
            self.user_input,
            self.image_paths,
            self.file_paths,
        )
    }
}

fn unanswered_output(lang: Lang) -> String {
    format!("{}\n{}", MARKER_STATUS, tr(lang, "status.unanswered"))
}

/// 按问题聚合「发送」路径的输出（取消路径请直接用 `cancel_output`）。
///
/// - 单题：现状格式（无 `# Q1` 头）；空回答 → 未作答状态。
/// - 多题：每题 `# Qn` + 区块，题间用 `---` 分隔；未答题为未作答状态；
///   全部未答 → 仅输出一次取消提示。
/// - 附注（spec message-annotations D7）：单题接在其余区块之后；多题放在最后一题之后，用 `---`
///   隔开、不带 `# Qn`；回答全空而有附注时只输出附注区块（既不是「未作答」，也不是「取消」）。
pub fn aggregate_output(
    lang: Lang,
    answers: &[RenderedAnswer],
    annotations: &[UserAnnotation],
) -> String {
    let notes = annotations::render_block(annotations);
    let nothing_answered = answers.iter().all(|a| a.is_empty());
    if !notes.is_empty() && nothing_answered {
        return notes;
    }

    if answers.len() <= 1 {
        let body = match answers.first() {
            Some(a) if !a.is_empty() => a.body(lang),
            _ => unanswered_output(lang),
        };
        return if notes.is_empty() {
            body
        } else {
            format!("{body}\n\n{notes}")
        };
    }

    if nothing_answered {
        return cancel_output(lang);
    }

    let mut parts: Vec<String> = answers
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let body = if a.is_empty() {
                unanswered_output(lang)
            } else {
                a.body(lang)
            };
            format!("# Q{}\n{}", i + 1, body)
        })
        .collect();
    if !notes.is_empty() {
        parts.push(notes);
    }
    parts.join("\n\n---\n\n")
}

/// 成功路径输出（图片已落盘，传入路径列表；文件为用户拖入的非图片绝对路径，直接透传）。
pub fn send_output(
    lang: Lang,
    selected_options: &[String],
    user_input: Option<&str>,
    image_paths: &[String],
    file_paths: &[String],
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !selected_options.is_empty() {
        sections.push(format!(
            "{}\n{}",
            MARKER_SELECTED_OPTIONS,
            selected_options.join(", ")
        ));
    }

    if let Some(input) = user_input {
        let trimmed = input.trim();
        if !trimmed.is_empty() {
            sections.push(format!("{}\n{}", MARKER_USER_INPUT, trimmed));
        }
    }

    // 图片落盘路径 + 透传文件路径，合并为单一 `[files]`（D6b：模型按后缀区分类型）。
    let files: Vec<&String> = image_paths.iter().chain(file_paths.iter()).collect();
    if !files.is_empty() {
        let joined = files
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("{}\n{}", MARKER_FILES, joined));
    }

    if sections.is_empty() {
        sections.push(format!(
            "{}\n{}",
            MARKER_USER_INPUT,
            tr(lang, "status.confirmContinue")
        ));
    }

    sections.join("\n\n")
}

/// whats-next 提交的语义映射结果（spec todo-whats-next D2 表格五分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhatsNextReply {
    /// 派活：任务文本（选中待办原文 ± 空行拼补充，或纯自由文本）→ `[user_input]`。
    Task(String),
    /// Caller-suggested next tasks selected by the human. Keep ordinary Ask semantics:
    /// the options stay under `[selected_options]` and an optional supplement stays under
    /// `[user_input]`.
    Suggested {
        options: Vec<String>,
        user_input: Option<String>,
    },
    /// 准许结束：携带「结束本轮」选项原文 → `[selected_options]`（第 19 轮定案：复用
    /// Ask 标准区块，不再输出固定英文结束句；agent 据 rules 语义判断可结束）。
    End(String),
    /// Notes on the message sent with an empty submission or with the end option (spec
    /// `message-annotations` D5): the end is not approved, the Agent only gets the notes (and any
    /// files) to answer, then asks again.
    NotesOnly,
    /// 未作答 / 取消：沿用普通 Ask 取消语义（`[status]` 指示继续询问）。
    Cancelled,
}

/// whats-next 提交映射（纯函数，spec todo-whats-next D2/D3）。
///
/// Todo choices become their raw task texts; caller suggestions preserve ordinary Ask option/input
/// blocks. The final end option approves ending only when no task or free text was submitted.
/// 出队不在此处：赢家回答的待办 id 由 `todos::ids_to_dequeue` 统一收集、Coordinator 出队。
///
/// Notes on the message change one thing: an end approval (or an empty submission) that carries
/// them becomes [`WhatsNextReply::NotesOnly`] — having something to say means the turn is not done.
pub fn whats_next_reply(request: &AskRequest, result: &ChannelResult) -> WhatsNextReply {
    let reply = whats_next_task_or_end(request, result);
    let has_notes = result.action == ChannelAction::Send && !result.annotations.is_empty();
    match reply {
        WhatsNextReply::End(_) | WhatsNextReply::Cancelled if has_notes => {
            WhatsNextReply::NotesOnly
        }
        other => other,
    }
}

/// The submission mapping without notes: task, suggestion, end approval or cancellation.
fn whats_next_task_or_end(request: &AskRequest, result: &ChannelResult) -> WhatsNextReply {
    if result.action == ChannelAction::Cancel {
        return WhatsNextReply::Cancelled;
    }
    let answer = result.answers.first();
    let input = answer
        .and_then(|a| a.user_input.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let options = request
        .questions
        .first()
        .map(|q| q.predefined_options.as_slice())
        .unwrap_or(&[]);
    let selected = answer.map(|a| a.selected_options.as_slice()).unwrap_or(&[]);
    // Preserve the displayed order, including every selected todo and its frozen text.
    let task_text = options
        .iter()
        .filter(|option| option.todo_id.is_some() && selected.contains(&option.text))
        .map(|todo| {
            todo.todo_text
                .as_deref()
                .unwrap_or_else(|| strip_todo_prefix(&todo.text))
        })
        .chain(input)
        .collect::<Vec<_>>()
        .join("\n\n");
    let end_index = options.len().saturating_sub(1);
    let suggestions: Vec<String> = options
        .iter()
        .take(end_index)
        .filter(|option| option.todo_id.is_none() && selected.contains(&option.text))
        .map(|option| option.text.clone())
        .collect();
    if !suggestions.is_empty() {
        return WhatsNextReply::Suggested {
            options: suggestions,
            user_input: Some(task_text).filter(|text| !text.is_empty()),
        };
    }
    if !task_text.is_empty() {
        return WhatsNextReply::Task(task_text);
    }
    let selected_end = options
        .last()
        .filter(|option| selected.contains(&option.text))
        .map(|option| option.text.clone());
    match selected_end {
        Some(end_text) => WhatsNextReply::End(end_text),
        None => WhatsNextReply::Cancelled,
    }
}

/// whats-next 的 stdout 渲染（spec D3，第 19 轮定案：复用 Ask 标准区块）。
/// 派活 → `[user_input]` + 任务文本；准许结束 → `[selected_options]` + 结束选项原文；
/// 取消沿用 `[status]`（从不带附注）。回答附带的图片/文件已由调用方落盘为路径，非空时按既有
/// `[files]` 区块附后（普通 Ask 同一契约）。附注（spec message-annotations D5）在 `[files]` 之后，
/// `NotesOnly` 只有 `[files]`（如有）与附注区块，没有「结束本轮」那行。
pub fn whats_next_output(
    reply: &WhatsNextReply,
    file_paths: &[String],
    annotations: &[UserAnnotation],
    lang: Lang,
) -> String {
    let base = match reply {
        WhatsNextReply::Task(text) => format!("{}\n{}", MARKER_USER_INPUT, text),
        WhatsNextReply::Suggested {
            options,
            user_input,
        } => send_output(lang, options, user_input.as_deref(), &[], &[]),
        WhatsNextReply::End(text) => format!("{}\n{}", MARKER_SELECTED_OPTIONS, text),
        WhatsNextReply::NotesOnly => String::new(),
        WhatsNextReply::Cancelled => return cancel_output(lang),
    };
    let mut sections: Vec<String> = Vec::new();
    if !base.is_empty() {
        sections.push(base);
    }
    if !file_paths.is_empty() {
        sections.push(format!("{}\n{}", MARKER_FILES, file_paths.join("\n")));
    }
    let notes = annotations::render_block(annotations);
    if !notes.is_empty() {
        sections.push(notes);
    }
    sections.join("\n\n")
}

/// 结构化 JSON 输出（D7）：snake_case、美化多行、省略空字段；`answers` 仅含**有作答**的题。
/// `image_paths_per_q` 为各题已落盘图片路径（与 `result.answers` 同序），与透传文件合并进 `files`。
pub fn render_json(
    request: &AskRequest,
    result: &ChannelResult,
    image_paths_per_q: &[Vec<String>],
    lang: Lang,
) -> String {
    #[derive(Serialize)]
    struct JsonOutput {
        action: &'static str,
        channel: String,
        /// 本次提问的状态说明（与文本侧 `[status]` 一致）：取消时是「必须重新确认」的引导，
        /// 重放旧回答时是重放说明（spec duplicate-ask-coalescing D7）。无特殊状态时省略。
        /// **判断是否被取消看 `action`**，不要用「有没有 status」来判断。
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        answers: Vec<JsonAnswer>,
        /// The user's notes on passages of the message, quotes whole. Absent when there are none;
        /// notes alone make an `answer` without `answers`.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        user_annotations: Vec<UserAnnotation>,
    }
    #[derive(Serialize)]
    struct JsonAnswer {
        question_index: usize,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        selected_options: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        selected_indices: Vec<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        user_input: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    }

    let action = match result.action {
        ChannelAction::Send => "answer",
        ChannelAction::Cancel => "cancel",
    };

    let mut answers: Vec<JsonAnswer> = Vec::new();
    if matches!(result.action, ChannelAction::Send) {
        for (i, ans) in result.answers.iter().enumerate() {
            let user_input = ans
                .user_input
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let images = image_paths_per_q.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let files: Vec<String> = images.iter().chain(ans.files.iter()).cloned().collect();

            // 跳过空回答（与文本侧 is_empty 一致：无选项 / 无输入 / 无附件）。
            if ans.selected_options.is_empty() && user_input.is_none() && files.is_empty() {
                continue;
            }

            // 选项原文 → 0 基下标（推荐前缀不进 selected_options，按原文匹配；重复取首个）。
            let opts = request
                .questions
                .get(i)
                .map(|q| q.predefined_options.as_slice())
                .unwrap_or(&[]);
            let selected_indices: Vec<usize> = ans
                .selected_options
                .iter()
                .filter_map(|sel| opts.iter().position(|o| &o.text == sel))
                .collect();

            answers.push(JsonAnswer {
                question_index: i,
                selected_options: ans.selected_options.clone(),
                selected_indices,
                user_input,
                files,
            });
        }
    }

    // 取消路径补上引导文案（D：MCP / 脚本据此重新确认，不把取消当默认放行）。
    let status = match result.action {
        ChannelAction::Cancel => Some(tr(lang, "status.cancel").to_string()),
        ChannelAction::Send => None,
    };

    let user_annotations = match result.action {
        ChannelAction::Send => result.annotations.clone(),
        ChannelAction::Cancel => Vec::new(),
    };

    let out = JsonOutput {
        action,
        channel: result.source_channel_id.clone(),
        status,
        answers,
        user_annotations,
    };
    serde_json::to_string_pretty(&out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    // 结构断言用英文（源语言）；字段标记恒英文，不随语言变化。

    #[test]
    fn auto_reply_text_is_a_status_block() {
        assert_eq!(auto_reply_output("go on"), "[status]\ngo on");
    }

    #[test]
    fn auto_reply_json_has_its_own_action_and_never_looks_like_an_answer() {
        let value: serde_json::Value =
            serde_json::from_str(&auto_reply_json("away", "go on")).unwrap();
        assert_eq!(value["action"], "auto_reply");
        assert_eq!(value["channel"], "away");
        assert_eq!(value["status"], "go on");
        assert!(value.get("answers").is_none());
    }

    #[test]
    fn append_status_follows_a_text_answer_in_its_own_block() {
        let out = append_status("[selected_options]\nA\n", "stop asking", false, None);
        assert_eq!(out, "[selected_options]\nA\n\n[status]\nstop asking");
        // With no answer at all only the status remains.
        assert_eq!(
            append_status("", "stop asking", false, None),
            "[status]\nstop asking"
        );
    }

    #[test]
    fn append_status_fills_the_json_status_field_and_keeps_an_existing_one() {
        let out = append_status(
            r#"{"action":"answer","answers":[]}"#,
            "stop asking",
            true,
            Some("mute"),
        );
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["action"], "answer");
        assert_eq!(value["status"], "stop asking");
        assert_eq!(value["directive"], "mute");
        let out = append_status(
            r#"{"action":"answer","status":"replayed"}"#,
            "stop asking",
            true,
            None,
        );
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["status"], "replayed\nstop asking");
        assert!(value.get("directive").is_none());
        // Broken JSON is left alone rather than made worse.
        assert_eq!(append_status("not json", "x", true, None), "not json");
    }

    #[test]
    fn options_only() {
        let out = send_output(Lang::En, &s(&["A", "B"]), None, &[], &[]);
        assert_eq!(out, "[selected_options]\nA, B");
    }

    #[test]
    fn input_trimmed() {
        let out = send_output(Lang::En, &[], Some("  hi  \n"), &[], &[]);
        assert_eq!(out, "[user_input]\nhi");
    }

    #[test]
    fn empty_input_omitted() {
        let out = send_output(Lang::En, &[], Some("   "), &[], &[]);
        assert_eq!(out, "[user_input]\nUser confirmed to continue");
    }

    #[test]
    fn images_render_as_files() {
        let out = send_output(Lang::En, &s(&["A"]), Some("hi"), &s(&["/tmp/a.png"]), &[]);
        assert_eq!(
            out,
            "[selected_options]\nA\n\n[user_input]\nhi\n\n[files]\n/tmp/a.png"
        );
    }

    #[test]
    fn images_and_files_merge_into_single_files_block() {
        let out = send_output(
            Lang::En,
            &[],
            Some("hi"),
            &s(&["/tmp/a.png"]),
            &s(&["/tmp/b.md"]),
        );
        assert_eq!(out, "[user_input]\nhi\n\n[files]\n/tmp/a.png\n/tmp/b.md");
    }

    #[test]
    fn empty_all_confirms_continue() {
        let out = send_output(Lang::En, &[], None, &[], &[]);
        assert_eq!(out, "[user_input]\nUser confirmed to continue");
    }

    #[test]
    fn markers_not_localized_in_zh() {
        // 字段标记恒英文；仅值文案随语言（此例值是用户输入，原样保留）。
        let out = send_output(Lang::Zh, &s(&["A"]), Some("你好"), &[], &[]);
        assert_eq!(out, "[selected_options]\nA\n\n[user_input]\n你好");
    }

    #[test]
    fn cancel_text() {
        assert!(cancel_output(Lang::En).starts_with("[status]\n"));
        assert!(cancel_output(Lang::Zh).starts_with("[status]\n"));
    }

    fn ans<'a>(
        opts: &'a [String],
        input: Option<&'a str>,
        imgs: &'a [String],
        files: &'a [String],
    ) -> RenderedAnswer<'a> {
        RenderedAnswer {
            selected_options: opts,
            user_input: input,
            image_paths: imgs,
            file_paths: files,
        }
    }

    #[test]
    fn single_answered_keeps_current_format() {
        let opts = s(&["A"]);
        let out = aggregate_output(Lang::En, &[ans(&opts, Some("hi"), &[], &[])], &[]);
        assert_eq!(out, "[selected_options]\nA\n\n[user_input]\nhi");
    }

    #[test]
    fn single_empty_is_unanswered() {
        let out = aggregate_output(Lang::En, &[ans(&[], Some("   "), &[], &[])], &[]);
        assert_eq!(out, "[status]\nThe user did not answer this question");
    }

    #[test]
    fn multi_all_answered_grouped() {
        let o1 = s(&["A"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&o1, None, &[], &[]), ans(&[], Some("ok"), &[], &[])],
            &[],
        );
        assert_eq!(
            out,
            "# Q1\n[selected_options]\nA\n\n---\n\n# Q2\n[user_input]\nok"
        );
    }

    #[test]
    fn multi_partial_unanswered() {
        let o1 = s(&["A"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&o1, None, &[], &[]), ans(&[], None, &[], &[])],
            &[],
        );
        assert_eq!(
            out,
            "# Q1\n[selected_options]\nA\n\n---\n\n# Q2\n[status]\nThe user did not answer this question"
        );
    }

    #[test]
    fn multi_all_unanswered_is_cancel() {
        let out = aggregate_output(
            Lang::En,
            &[ans(&[], None, &[], &[]), ans(&[], Some(" "), &[], &[])],
            &[],
        );
        assert_eq!(out, cancel_output(Lang::En));
    }

    // ===== render_json =====

    use crate::models::{MessagePrompt, OptionItem, OutputFormat, Question, QuestionAnswer};

    fn req(questions: Vec<Question>) -> AskRequest {
        let mut r = AskRequest::new(MessagePrompt::default(), questions, true);
        r.output_format = OutputFormat::Json;
        r
    }

    fn q(opts: &[&str]) -> Question {
        Question::new(
            "Q".into(),
            opts.iter().map(|t| OptionItem::new(*t, false)).collect(),
        )
    }

    fn answered(opts: &[&str], input: Option<&str>, files: &[&str]) -> QuestionAnswer {
        QuestionAnswer {
            selected_options: opts.iter().map(|x| x.to_string()).collect(),
            user_input: input.map(|s| s.to_string()),
            images: Vec::new(),
            files: files.iter().map(|x| x.to_string()).collect(),
            todo_ids: Vec::new(),
            todo_selections: Vec::new(),
        }
    }

    #[test]
    fn json_answer_maps_indices_and_omits_empty_fields() {
        let request = req(vec![q(&["staging", "production"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![answered(&["production"], None, &[])],
            source_channel_id: "popup".into(),
            annotations: Vec::new(),
        };
        let json = render_json(&request, &result, &[vec![]], Lang::En);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["action"], "answer");
        assert_eq!(v["channel"], "popup");
        assert_eq!(v["answers"][0]["question_index"], 0);
        assert_eq!(v["answers"][0]["selected_options"][0], "production");
        assert_eq!(v["answers"][0]["selected_indices"][0], 1);
        // 未填字段应省略。
        assert!(v["answers"][0].get("user_input").is_none());
        assert!(v["answers"][0].get("files").is_none());
        // 正常作答不应出现 status。
        assert!(v.get("status").is_none());
    }

    #[test]
    fn json_skips_empty_answers_keeps_original_index() {
        let request = req(vec![q(&["A"]), q(&["B"]), q(&["C"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![
                QuestionAnswer::default(),   // Q0 未答
                answered(&["B"], None, &[]), // Q1 已答
                QuestionAnswer::default(),   // Q2 未答
            ],
            source_channel_id: "slack".into(),
            annotations: Vec::new(),
        };
        let v: serde_json::Value = serde_json::from_str(&render_json(
            &request,
            &result,
            &[vec![], vec![], vec![]],
            Lang::En,
        ))
        .unwrap();
        assert_eq!(v["answers"].as_array().unwrap().len(), 1);
        assert_eq!(v["answers"][0]["question_index"], 1);
    }

    #[test]
    fn json_merges_images_and_files() {
        let request = req(vec![q(&["A"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![answered(&[], Some("note"), &["/tmp/b.md"])],
            source_channel_id: "telegram".into(),
            annotations: Vec::new(),
        };
        let v: serde_json::Value = serde_json::from_str(&render_json(
            &request,
            &result,
            &[vec!["/tmp/a.png".into()]],
            Lang::En,
        ))
        .unwrap();
        let files = v["answers"][0]["files"].as_array().unwrap();
        assert_eq!(files[0], "/tmp/a.png");
        assert_eq!(files[1], "/tmp/b.md");
        assert_eq!(v["answers"][0]["user_input"], "note");
    }

    // ===== whats-next（spec todo-whats-next D2/D3）=====

    /// whats-next request: suggestions, todo chips, then the final end option.
    fn wn_request() -> AskRequest {
        let mut r = AskRequest::new(
            MessagePrompt::default(),
            vec![Question::new(
                "What should we do next?".into(),
                vec![
                    OptionItem::new("Write docs", false),
                    OptionItem::new("Add tests", true),
                    OptionItem::with_todo("执行待办：修复登录 bug", "id-1"),
                    OptionItem::with_todo("Run todo: 写发布说明", "id-2"),
                    OptionItem::new("End this turn", false),
                ],
            )],
            true,
        );
        r.single = false;
        r.whats_next = true;
        r
    }

    fn wn_result(selected: &[&str], input: Option<&str>) -> ChannelResult {
        ChannelResult {
            action: ChannelAction::Send,
            answers: vec![answered(selected, input, &[])],
            source_channel_id: "popup".into(),
            annotations: Vec::new(),
        }
    }

    #[test]
    fn whats_next_selected_todo_outputs_its_text_without_prefix() {
        // 选项带「执行待办：」展示前缀（中/英），任务文本还原为待办原文。
        let reply = whats_next_reply(&wn_request(), &wn_result(&["执行待办：修复登录 bug"], None));
        assert_eq!(reply, WhatsNextReply::Task("修复登录 bug".into()));
    }

    #[test]
    fn whats_next_todo_with_supplement_joins_with_blank_line() {
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(&["Run todo: 写发布说明"], Some(" 顺带更新截图 ")),
        );
        assert_eq!(
            reply,
            WhatsNextReply::Task("写发布说明\n\n顺带更新截图".into())
        );
    }

    #[test]
    fn whats_next_free_text_only_is_a_new_task() {
        let reply = whats_next_reply(&wn_request(), &wn_result(&[], Some("先跑一遍测试")));
        assert_eq!(reply, WhatsNextReply::Task("先跑一遍测试".into()));
    }

    #[test]
    fn whats_next_suggestion_keeps_ask_option_and_input_blocks() {
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(&["Add tests"], Some(" Cover the edge case ")),
        );
        assert_eq!(
            reply,
            WhatsNextReply::Suggested {
                options: vec!["Add tests".into()],
                user_input: Some("Cover the edge case".into()),
            }
        );
        assert_eq!(
            whats_next_output(&reply, &s(&["/tmp/report.md"]), &[], Lang::En),
            "[selected_options]\nAdd tests\n\n[user_input]\nCover the edge case\n\n[files]\n/tmp/report.md"
        );
    }

    #[test]
    fn whats_next_keeps_all_suggestions_todos_and_supplement() {
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(
                &[
                    "Add tests",
                    "Run todo: 写发布说明",
                    "Write docs",
                    "执行待办：修复登录 bug",
                    "End this turn",
                ],
                Some(" Also check CI "),
            ),
        );
        assert_eq!(
            reply,
            WhatsNextReply::Suggested {
                options: vec!["Write docs".into(), "Add tests".into()],
                user_input: Some("修复登录 bug\n\n写发布说明\n\nAlso check CI".into()),
            }
        );
        assert_eq!(whats_next_output(&reply, &[], &[], Lang::En),
            "[selected_options]\nWrite docs, Add tests\n\n[user_input]\n修复登录 bug\n\n写发布说明\n\nAlso check CI");
    }

    #[test]
    fn whats_next_keeps_every_selected_todo() {
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(&["Run todo: 写发布说明", "执行待办：修复登录 bug"], None),
        );
        assert_eq!(
            reply,
            WhatsNextReply::Task("修复登录 bug\n\n写发布说明".into())
        );
    }

    #[test]
    fn whats_next_a_task_selection_prevents_an_end_approval() {
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(&["End this turn", "Add tests"], None),
        );
        assert_eq!(
            reply,
            WhatsNextReply::Suggested {
                options: vec!["Add tests".into()],
                user_input: None
            }
        );
    }

    #[test]
    fn whats_next_end_without_text_approves_ending() {
        // 第 19 轮定案：结束 → `[selected_options]` + 结束选项原文（复用 Ask 标准区块）。
        let reply = whats_next_reply(&wn_request(), &wn_result(&["End this turn"], None));
        assert_eq!(reply, WhatsNextReply::End("End this turn".into()));
        assert_eq!(
            whats_next_output(&reply, &[], &[], Lang::En),
            "[selected_options]\nEnd this turn"
        );
    }

    #[test]
    fn whats_next_end_with_text_means_continue() {
        // 「结束＋文字」＝继续：文字是新指令（spec D2 表第 4 行）。
        let reply = whats_next_reply(
            &wn_request(),
            &wn_result(&["End this turn"], Some("还有个想法")),
        );
        assert_eq!(reply, WhatsNextReply::Task("还有个想法".into()));
    }

    #[test]
    fn whats_next_cancel_and_empty_submit_keep_asking() {
        let cancel = ChannelResult::cancel("popup");
        assert_eq!(
            whats_next_reply(&wn_request(), &cancel),
            WhatsNextReply::Cancelled
        );
        // 空提交（无选择无文本）同样视为未作答。
        assert_eq!(
            whats_next_reply(&wn_request(), &wn_result(&[], None)),
            WhatsNextReply::Cancelled
        );
        assert!(
            whats_next_output(&WhatsNextReply::Cancelled, &[], &[], Lang::En)
                .starts_with("[status]\n")
        );
    }

    #[test]
    fn whats_next_output_appends_files_block() {
        // 派活 → `[user_input]` + 任务文本（第 19 轮定案）；文件照旧 `[files]` 附后。
        let reply = WhatsNextReply::Task("任务".into());
        assert_eq!(
            whats_next_output(&reply, &s(&["/tmp/a.png"]), &[], Lang::En),
            "[user_input]\n任务\n\n[files]\n/tmp/a.png"
        );
        assert_eq!(
            whats_next_output(&reply, &[], &[], Lang::En),
            "[user_input]\n任务"
        );
    }

    #[test]
    fn json_cancel_has_no_answers() {
        let request = req(vec![q(&["A"])]);
        let result = ChannelResult::cancel("popup");
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[], Lang::En)).unwrap();
        assert_eq!(v["action"], "cancel");
        assert_eq!(v["channel"], "popup");
        assert!(v.get("answers").is_none());
        // 取消时必须带引导文案，要求模型重新确认。
        assert!(v["status"].as_str().unwrap().contains("must ask again"));
    }

    // ===== notes on the message (spec message-annotations) =====

    fn note(quote: &str, text: &str) -> UserAnnotation {
        UserAnnotation {
            quote: quote.into(),
            note: text.into(),
        }
    }

    fn two_notes() -> Vec<UserAnnotation> {
        vec![
            note("需要同时调整三处表单", "是哪三处？"),
            note("回归测试也已补充", "补在哪个文件里？"),
        ]
    }

    const TWO_NOTES_BLOCK: &str =
        "[user_annotations]\n> 需要同时调整三处表单\n是哪三处？\n\n> 回归测试也已补充\n补在哪个文件里？";

    #[test]
    fn a_single_answer_gets_the_notes_block_after_its_other_blocks() {
        let opts = s(&["方案 2"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&opts, Some("先按这个做"), &[], &s(&["/tmp/a.md"]))],
            &two_notes(),
        );
        assert_eq!(
            out,
            format!(
                "[selected_options]\n方案 2\n\n[user_input]\n先按这个做\n\n[files]\n/tmp/a.md\n\n{TWO_NOTES_BLOCK}"
            )
        );
    }

    #[test]
    fn notes_alone_are_neither_unanswered_nor_a_cancel() {
        // One question.
        let out = aggregate_output(Lang::En, &[ans(&[], Some("  "), &[], &[])], &two_notes());
        assert_eq!(out, TWO_NOTES_BLOCK);
        // Several questions: no `# Qn`, no status lines.
        let out = aggregate_output(
            Lang::En,
            &[ans(&[], None, &[], &[]), ans(&[], None, &[], &[])],
            &two_notes(),
        );
        assert_eq!(out, TWO_NOTES_BLOCK);
        // No answers at all (nothing to render) is the same.
        assert_eq!(
            aggregate_output(Lang::En, &[], &two_notes()),
            TWO_NOTES_BLOCK
        );
    }

    #[test]
    fn several_questions_put_the_notes_after_the_last_one_behind_a_rule() {
        let o1 = s(&["独立会话表"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&o1, None, &[], &[]), ans(&[], None, &[], &[])],
            &[note(
                "线上还有约 1.2 万条旧会话",
                "这些里面还在活跃的有多少？",
            )],
        );
        assert_eq!(
            out,
            "# Q1\n[selected_options]\n独立会话表\n\n---\n\n# Q2\n[status]\nThe user did not answer this question\n\n---\n\n[user_annotations]\n> 线上还有约 1.2 万条旧会话\n这些里面还在活跃的有多少？"
        );
    }

    #[test]
    fn no_notes_leave_every_text_output_as_it_was() {
        let o1 = s(&["A"]);
        assert_eq!(
            aggregate_output(Lang::En, &[ans(&o1, None, &[], &[])], &[]),
            "[selected_options]\nA"
        );
        assert_eq!(
            aggregate_output(Lang::En, &[ans(&[], None, &[], &[])], &[]),
            unanswered_output(Lang::En)
        );
    }

    #[test]
    fn a_long_quote_is_shortened_in_the_text_block_only() {
        let long = "字".repeat(500);
        let notes = [note(&long, "why")];
        let out = aggregate_output(Lang::En, &[ans(&[], None, &[], &[])], &notes);
        let quote_line = out.lines().nth(1).unwrap();
        assert_eq!(quote_line.chars().count(), 2 + 300);
        assert!(quote_line.contains('…'));
        // JSON keeps the whole quote.
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![QuestionAnswer::default()],
            source_channel_id: "popup".into(),
            annotations: notes.to_vec(),
        };
        let v: serde_json::Value = serde_json::from_str(&render_json(
            &req(vec![q(&[])]),
            &result,
            &[vec![]],
            Lang::En,
        ))
        .unwrap();
        assert_eq!(v["user_annotations"][0]["quote"], long);
    }

    #[test]
    fn a_status_text_still_comes_last() {
        let out = aggregate_output(Lang::En, &[ans(&[], None, &[], &[])], &two_notes());
        let with_status = append_status(&out, "stop asking", false, None);
        assert_eq!(
            with_status,
            format!("{TWO_NOTES_BLOCK}\n\n[status]\nstop asking")
        );
    }

    #[test]
    fn json_lists_the_notes_and_notes_alone_are_an_answer_without_answers() {
        let request = req(vec![q(&["A"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![QuestionAnswer::default()],
            source_channel_id: "popup".into(),
            annotations: two_notes(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[vec![]], Lang::En)).unwrap();
        assert_eq!(v["action"], "answer");
        assert!(v.get("answers").is_none());
        assert!(v.get("status").is_none());
        assert_eq!(
            v["user_annotations"],
            serde_json::json!([
                {"quote": "需要同时调整三处表单", "note": "是哪三处？"},
                {"quote": "回归测试也已补充", "note": "补在哪个文件里？"},
            ])
        );
        // Next to a real answer the notes sit beside it, with no question index of their own.
        let result = ChannelResult {
            answers: vec![answered(&["A"], None, &[])],
            ..result
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[vec![]], Lang::En)).unwrap();
        assert_eq!(v["answers"][0]["selected_options"][0], "A");
        assert!(v["user_annotations"][0].get("question_index").is_none());
        // Without notes the key is absent; a cancellation never carries them.
        let plain = ChannelResult {
            annotations: Vec::new(),
            ..result.clone()
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &plain, &[vec![]], Lang::En)).unwrap();
        assert!(v.get("user_annotations").is_none());
        let cancelled = ChannelResult {
            action: ChannelAction::Cancel,
            ..result
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &cancelled, &[], Lang::En)).unwrap();
        assert_eq!(v["action"], "cancel");
        assert!(v.get("user_annotations").is_none());
    }

    fn wn_result_with_notes(selected: &[&str], input: Option<&str>) -> ChannelResult {
        ChannelResult {
            annotations: two_notes(),
            ..wn_result(selected, input)
        }
    }

    #[test]
    fn whats_next_end_or_an_empty_submit_with_notes_does_not_approve_ending() {
        let end = whats_next_reply(
            &wn_request(),
            &wn_result_with_notes(&["End this turn"], None),
        );
        assert_eq!(end, WhatsNextReply::NotesOnly);
        let empty = whats_next_reply(&wn_request(), &wn_result_with_notes(&[], None));
        assert_eq!(empty, WhatsNextReply::NotesOnly);
        // No "End this turn" line: only the notes, after the files when there are any.
        assert_eq!(
            whats_next_output(&end, &[], &two_notes(), Lang::En),
            TWO_NOTES_BLOCK
        );
        assert_eq!(
            whats_next_output(&end, &s(&["/tmp/a.png"]), &two_notes(), Lang::En),
            format!("[files]\n/tmp/a.png\n\n{TWO_NOTES_BLOCK}")
        );
    }

    #[test]
    fn whats_next_tasks_and_suggestions_keep_their_output_and_add_the_notes_last() {
        let task = whats_next_reply(
            &wn_request(),
            &wn_result_with_notes(&["End this turn"], Some("还有个想法")),
        );
        assert_eq!(task, WhatsNextReply::Task("还有个想法".into()));
        assert_eq!(
            whats_next_output(&task, &s(&["/tmp/a.png"]), &two_notes(), Lang::En),
            format!("[user_input]\n还有个想法\n\n[files]\n/tmp/a.png\n\n{TWO_NOTES_BLOCK}")
        );
        let suggested =
            whats_next_reply(&wn_request(), &wn_result_with_notes(&["Add tests"], None));
        assert!(matches!(suggested, WhatsNextReply::Suggested { .. }));
        assert_eq!(
            whats_next_output(&suggested, &[], &two_notes(), Lang::En),
            format!("[selected_options]\nAdd tests\n\n{TWO_NOTES_BLOCK}")
        );
        let todo = whats_next_reply(
            &wn_request(),
            &wn_result_with_notes(&["执行待办：修复登录 bug"], None),
        );
        assert_eq!(todo, WhatsNextReply::Task("修复登录 bug".into()));
    }

    #[test]
    fn whats_next_cancel_never_carries_notes() {
        let cancelled = ChannelResult {
            action: ChannelAction::Cancel,
            ..wn_result_with_notes(&[], None)
        };
        let reply = whats_next_reply(&wn_request(), &cancelled);
        assert_eq!(reply, WhatsNextReply::Cancelled);
        assert!(whats_next_output(&reply, &[], &two_notes(), Lang::En).starts_with("[status]\n"));
        assert!(
            !whats_next_output(&reply, &[], &two_notes(), Lang::En).contains("[user_annotations]")
        );
    }
}
