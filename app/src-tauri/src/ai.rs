//! AI 总结（周/月回顾的第二层）
//!
//! 设计约束，按重要性排：
//! 1. **没网/没 key/模型报错都必须还能出总结** —— 所以本模块只在
//!    `stats::digest` 已算好的本地摘要之上做润色，绝不做唯一数据源。
//! 2. **不擅自读系统里的其它凭据**。key 由用户在设置里填，走 OpenAI 兼容
//!    接口，默认预填阿里云百炼（千问）的兼容端点 —— 用户说"用现在用的
//!    千问模型"，但那是 IDE 的 DeepSeek 凭据，与千问无关，我不会挪用。
//! 3. 请求构造与响应解析是纯函数，可单测；只有 `ask` 真发网络。
//!
//! ⚠️ key 明文存在本机 store.json（与用户数据同目录同权限）。这是本机桌面
//! 应用的常见做法（Kite 的 WebDAV 口令同样落本地），但要在 UI 上说明白。

use serde::Deserialize;

/// 请求体。`response_format` 让模型只回 JSON 在本类任务上不稳，
/// 我们只要一段中文短文，纯文本更好处理。
pub fn build_body(model: &str, digest: &str, period_label: &str) -> String {
    let sys = "你是一个时间管理助手。根据用户的待办统计数据，写一段简洁、具体、\
               第二人称的中文回顾。要求：不超过 150 字；只依据给出的数字，\
               不要编造未提供的事实；先说结论（这段时间完成/遗留情况），\
               再点出一个值得注意的模式（如某天产出最高、某清单积压、\
               环比变化），最后一句给出下一步建议。不要列表、不要标题、不要客套话。";
    let user = format!("{period_label}\n以下是统计数据：\n{digest}");
    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": sys },
            { "role": "user", "content": user },
        ],
        // 回顾类任务要稳定，温度压低
        "temperature": 0.4,
        "max_tokens": 400,
        "stream": false,
    });
    body.to_string()
}

/// 拼 URL：baseUrl 可能带或不带尾斜杠、可能已含 /chat/completions
pub fn chat_url(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
}

#[derive(Deserialize)]
struct ChatReply {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ReplyMessage,
}

#[derive(Deserialize)]
struct ReplyMessage {
    content: Option<String>,
}

/// 解析响应。模型偶尔会在正文前后加引号或空行，这里一并清掉。
pub fn parse_reply(json: &str) -> Result<String, String> {
    let r: ChatReply =
        serde_json::from_str(json).map_err(|e| format!("模型返回格式异常：{e}"))?;
    let text = r
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "模型没返回内容".to_string())?;
    let cleaned = text.trim().trim_matches('"').trim().to_string();
    if cleaned.is_empty() {
        return Err("模型返回空内容".into());
    }
    Ok(cleaned)
}

/// 真发请求。超时 25s：总结是用户主动点的，可以等，但不能无限等。
/// 任何失败都返回 Err，由调用方降级到本地摘要。
///
/// 不接 `model` 参数：模型名已在 `build_body` 里编进请求体，这里再传一次
/// 就是两个真源，早晚漂移。
pub fn ask(base_url: &str, api_key: &str, body: &str) -> Result<String, String> {
    let url = chat_url(base_url);
    // ureq 3：超时挂在 Agent 配置上（2.x 的 RequestBuilder::timeout 已不存在）
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(25)))
        .build()
        .new_agent();
    let resp = agent
        .post(&url)
        .content_type("application/json")
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .send(body)
        .map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status().as_u16();
    let mut resp = resp;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("读取响应失败：{e}"))?;
    if !(200..300).contains(&status) {
        // 截断：错误体可能很大（HTML 错误页），且可能含回显的请求头
        let brief: String = text.chars().take(200).collect();
        return Err(format!("HTTP {status}：{brief}"));
    }
    parse_reply(&text)
}

/// 配置是否够用（够到值得发请求）
pub fn ready(base_url: &str, api_key: &str, model: &str) -> bool {
    !base_url.trim().is_empty() && !api_key.trim().is_empty() && !model.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalizes_trailing_slash_and_suffix() {
        assert_eq!(
            chat_url("https://dashscope.aliyuncs.com/compatible-mode/v1"),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions",
            "尾斜杠不该产出双斜杠"
        );
        assert_eq!(
            chat_url("https://x.test/v1/chat/completions"),
            "https://x.test/v1/chat/completions",
            "已含后缀不该重复"
        );
    }

    #[test]
    fn body_carries_model_and_digest_and_is_valid_json() {
        let body = build_body("qwen-plus", "完成 5 项\n遗留 2 项", "本周（2026-03-09 ~ 2026-03-15）");
        let v: serde_json::Value = serde_json::from_str(&body).expect("必须是合法 JSON");
        assert_eq!(v["model"], "qwen-plus");
        assert_eq!(v["messages"].as_array().unwrap().len(), 2);
        assert!(v["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("完成 5 项"));
        assert!(v["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("本周"));
        assert_eq!(v["stream"], false);
    }

    #[test]
    fn parses_openai_shape() {
        let ok = r#"{"choices":[{"message":{"role":"assistant","content":"  这周完成度不错。  "}}]}"#;
        assert_eq!(parse_reply(ok).unwrap(), "这周完成度不错。", "应 trim");
    }

    #[test]
    fn strips_wrapping_quotes() {
        let ok = r#"{"choices":[{"message":{"content":"\"带引号的回答\""}}]}"#;
        assert_eq!(parse_reply(ok).unwrap(), "带引号的回答");
    }

    #[test]
    fn errors_instead_of_panicking_on_bad_payloads() {
        // 非 JSON
        assert!(parse_reply("<html>502</html>").is_err());
        // 没 choices
        assert!(parse_reply(r#"{"error":{"message":"bad key"}}"#).is_err());
        // choices 空数组
        assert!(parse_reply(r#"{"choices":[]}"#).is_err());
        // content 为 null（部分网关的流式残留形态）
        assert!(parse_reply(r#"{"choices":[{"message":{"content":null}}]}"#).is_err());
        // content 全空白
        assert!(parse_reply(r#"{"choices":[{"message":{"content":"   "}}]}"#).is_err());
    }

    #[test]
    fn ready_requires_all_three() {
        assert!(!ready("", "sk-x", "qwen-plus"), "缺 base");
        assert!(!ready("https://x", "", "qwen-plus"), "缺 key = 不该发请求");
        assert!(!ready("https://x", "sk-x", ""), "缺 model");
        assert!(!ready("https://x", "   ", "qwen-plus"), "纯空格 key 不算有");
        assert!(ready("https://x", "sk-x", "qwen-plus"));
    }
}
