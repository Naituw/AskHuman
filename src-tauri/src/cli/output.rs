//! 结果区块格式化（区块结构固定，字段标记恒英文、不本地化）。
//!
//! 字段标记（`[selected_options]` 等）为固定英文常量（D6），与 `--agent-help`/`--scripting-help`
//! 共用同一常量，保证「AI 看到的实际输出」与 help 文档一致；仅**值文案**（取消/未作答/确认继续）
//! 仍随界面语言本地化。结构（分组顺序/`# Qn`/`---`）不随语言变化。

use crate::i18n::{tr, Lang};
use crate::models::{AskRequest, ChannelAction, ChannelResult};
use serde::Serialize;

/// 字段标记（恒英文，不本地化）。图片与文件已合并为单一 `[files]`（D6b）。
pub const MARKER_SELECTED_OPTIONS: &str = "[selected_options]";
pub const MARKER_USER_INPUT: &str = "[user_input]";
pub const MARKER_FILES: &str = "[files]";
pub const MARKER_STATUS: &str = "[status]";

/// 取消路径输出。
pub fn cancel_output(lang: Lang) -> String {
    format!("{}\n{}", MARKER_STATUS, tr(lang, "status.cancel"))
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
            && self
                .user_input
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
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
pub fn aggregate_output(lang: Lang, answers: &[RenderedAnswer]) -> String {
    if answers.len() <= 1 {
        return match answers.first() {
            Some(a) if !a.is_empty() => a.body(lang),
            _ => unanswered_output(lang),
        };
    }

    if answers.iter().all(|a| a.is_empty()) {
        return cancel_output(lang);
    }

    answers
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
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
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
        sections.push(format!("{}\n{}", MARKER_SELECTED_OPTIONS, selected_options.join(", ")));
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
        let joined = files.iter().map(|p| p.as_str()).collect::<Vec<_>>().join("\n");
        sections.push(format!("{}\n{}", MARKER_FILES, joined));
    }

    if sections.is_empty() {
        sections.push(format!("{}\n{}", MARKER_USER_INPUT, tr(lang, "status.confirmContinue")));
    }

    sections.join("\n\n")
}

/// 结构化 JSON 输出（D7）：snake_case、美化多行、省略空字段；`answers` 仅含**有作答**的题。
/// `image_paths_per_q` 为各题已落盘图片路径（与 `result.answers` 同序），与透传文件合并进 `files`。
pub fn render_json(
    request: &AskRequest,
    result: &ChannelResult,
    image_paths_per_q: &[Vec<String>],
) -> String {
    #[derive(Serialize)]
    struct JsonOutput {
        action: &'static str,
        channel: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        answers: Vec<JsonAnswer>,
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
            let files: Vec<String> =
                images.iter().chain(ans.files.iter()).cloned().collect();

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

    let out = JsonOutput {
        action,
        channel: result.source_channel_id.clone(),
        answers,
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
        assert_eq!(out, "[selected_options]\nA\n\n[user_input]\nhi\n\n[files]\n/tmp/a.png");
    }

    #[test]
    fn images_and_files_merge_into_single_files_block() {
        let out = send_output(Lang::En, &[], Some("hi"), &s(&["/tmp/a.png"]), &s(&["/tmp/b.md"]));
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
        let out = aggregate_output(Lang::En, &[ans(&opts, Some("hi"), &[], &[])]);
        assert_eq!(out, "[selected_options]\nA\n\n[user_input]\nhi");
    }

    #[test]
    fn single_empty_is_unanswered() {
        let out = aggregate_output(Lang::En, &[ans(&[], Some("   "), &[], &[])]);
        assert_eq!(out, "[status]\nThe user did not answer this question");
    }

    #[test]
    fn multi_all_answered_grouped() {
        let o1 = s(&["A"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&o1, None, &[], &[]), ans(&[], Some("ok"), &[], &[])],
        );
        assert_eq!(out, "# Q1\n[selected_options]\nA\n\n---\n\n# Q2\n[user_input]\nok");
    }

    #[test]
    fn multi_partial_unanswered() {
        let o1 = s(&["A"]);
        let out = aggregate_output(
            Lang::En,
            &[ans(&o1, None, &[], &[]), ans(&[], None, &[], &[])],
        );
        assert_eq!(
            out,
            "# Q1\n[selected_options]\nA\n\n---\n\n# Q2\n[status]\nThe user did not answer this question"
        );
    }

    #[test]
    fn multi_all_unanswered_is_cancel() {
        let out =
            aggregate_output(Lang::En, &[ans(&[], None, &[], &[]), ans(&[], Some(" "), &[], &[])]);
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
        }
    }

    #[test]
    fn json_answer_maps_indices_and_omits_empty_fields() {
        let request = req(vec![q(&["staging", "production"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![answered(&["production"], None, &[])],
            source_channel_id: "popup".into(),
        };
        let json = render_json(&request, &result, &[vec![]]);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["action"], "answer");
        assert_eq!(v["channel"], "popup");
        assert_eq!(v["answers"][0]["question_index"], 0);
        assert_eq!(v["answers"][0]["selected_options"][0], "production");
        assert_eq!(v["answers"][0]["selected_indices"][0], 1);
        // 未填字段应省略。
        assert!(v["answers"][0].get("user_input").is_none());
        assert!(v["answers"][0].get("files").is_none());
    }

    #[test]
    fn json_skips_empty_answers_keeps_original_index() {
        let request = req(vec![q(&["A"]), q(&["B"]), q(&["C"])]);
        let result = ChannelResult {
            action: ChannelAction::Send,
            answers: vec![
                QuestionAnswer::default(),     // Q0 未答
                answered(&["B"], None, &[]),   // Q1 已答
                QuestionAnswer::default(),     // Q2 未答
            ],
            source_channel_id: "slack".into(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[vec![], vec![], vec![]])).unwrap();
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
        };
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[vec!["/tmp/a.png".into()]]))
                .unwrap();
        let files = v["answers"][0]["files"].as_array().unwrap();
        assert_eq!(files[0], "/tmp/a.png");
        assert_eq!(files[1], "/tmp/b.md");
        assert_eq!(v["answers"][0]["user_input"], "note");
    }

    #[test]
    fn json_cancel_has_no_answers() {
        let request = req(vec![q(&["A"])]);
        let result = ChannelResult::cancel("popup");
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&request, &result, &[])).unwrap();
        assert_eq!(v["action"], "cancel");
        assert_eq!(v["channel"], "popup");
        assert!(v.get("answers").is_none());
    }
}
