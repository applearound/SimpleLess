use crate::config::PolishMode;
use serde::Deserialize;

/// 百炼 OpenAI 兼容端点，旧域名仍可正常使用
const API_BASE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

const POLISH_SYSTEM: &str = "你是一个语音输入润色助手。用户给你的文本来自语音识别，可能包含口头语气词、重复、半截句子和明显的口误。请把它整理成通顺的书面语：去掉语气词和重复，修正口误，保留原意与原有语言，不增删实质内容，不添加任何解释或前后缀。直接输出润色后的文本。若输入为空或没有实际内容，只输出空字符串。";

/// 把识别原文润色为书面语
pub async fn polish(key: &str, model: &str, text: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.3,
        "max_tokens": 2048,
        "enable_thinking": false,
        "messages": [
            { "role": "system", "content": POLISH_SYSTEM },
            { "role": "user", "content": text }
        ]
    });
    let content = chat(key, body).await?;
    Ok(content.trim().to_string())
}

/// 命令通道的执行结果
pub enum CommandOutcome {
    /// 润色档位已切换，附带给用户看的一句话确认
    ModeSwitched { mode: PolishMode, reply: String },
    /// 纯文本答复（如询问当前配置）
    Replied(String),
}

/// 把命令通道的语音转写给大模型做意图路由
pub async fn route_command(
    key: &str,
    model: &str,
    text: &str,
    polish_mode: PolishMode,
) -> Result<CommandOutcome, String> {
    let mode_desc = match polish_mode {
        PolishMode::Raw => "raw（原文：识别结果直接插入，不做润色）",
        PolishMode::Polished => "polished（润色：识别结果先整理成书面语再插入）",
    };
    let system = format!(
        "你是 SimpleLess 语音输入软件的配置助手，用户正在通过语音对你下达指令。\
当前配置：润色档位 = {mode_desc}。\
可用工具：set_polish_mode 可切换润色档位（raw 或 polished）。\
规则：用户想改设置就调用工具；用户询问配置就用一句话简短回答；\
如果用户的话不像是对软件说的指令，就原样返回这段文字，不做任何修改和解释。\
所有回复必须简短，不超过一句话。"
    );

    let body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 512,
        "enable_thinking": false,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": text }
        ],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "set_polish_mode",
                    "description": "切换听写文本的润色档位",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "mode": {
                                "type": "string",
                                "enum": ["raw", "polished"],
                                "description": "raw 为直接插入原文，polished 为先润色再插入"
                            }
                        },
                        "required": ["mode"]
                    }
                }
            }
        ],
        "tool_choice": "auto"
    });

    #[derive(Deserialize)]
    struct ToolCall {
        function: ToolFunction,
    }
    #[derive(Deserialize)]
    struct ToolFunction {
        arguments: String,
    }
    #[derive(Deserialize)]
    struct Message {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        tool_calls: Option<Vec<ToolCall>>,
    }
    #[derive(Deserialize)]
    struct Choice {
        message: Message,
    }
    #[derive(Deserialize)]
    struct Resp {
        choices: Vec<Choice>,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{API_BASE}/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用大模型失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("大模型返回错误 {status}: {truncate(&body)}"));
    }

    let parsed: Resp = resp
        .json()
        .await
        .map_err(|e| format!("解析大模型响应失败: {e}"))?;

    let message = parsed
        .choices
        .into_iter()
        .next()
        .ok_or("大模型未返回任何结果")?
        .message;

    if let Some(calls) = message.tool_calls {
        if let Some(call) = calls.into_iter().next() {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
            let mode = args["mode"].as_str().unwrap_or("");
            let new_mode = match mode {
                "raw" => Some(PolishMode::Raw),
                "polished" => Some(PolishMode::Polished),
                _ => None,
            };
            return match new_mode {
                Some(m) => Ok(CommandOutcome::ModeSwitched {
                    reply: format!("已切换：{}", m.label()),
                    mode: m,
                }),
                None => Ok(CommandOutcome::Replied("没听懂要切换到哪个档位".into())),
            };
        }
    }

    Ok(CommandOutcome::Replied(
        message.content.unwrap_or_default().trim().to_string(),
    ))
}

/// 用一次极小的请求验证 API Key 可用
pub async fn verify_key(key: &str, model: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 8,
        "enable_thinking": false,
        "messages": [{ "role": "user", "content": "hi" }]
    });
    chat(key, body).await.map(|_| ())
}

async fn chat(key: &str, body: serde_json::Value) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{API_BASE}/chat/completions"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用大模型失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("大模型返回错误 {status}: {truncate(&body)}"));
    }

    #[derive(Deserialize)]
    struct Resp {
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        message: Message,
    }
    #[derive(Deserialize)]
    struct Message {
        #[serde(default)]
        content: Option<String>,
    }

    let parsed: Resp = resp
        .json()
        .await
        .map_err(|e| format!("解析大模型响应失败: {e}"))?;
    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .ok_or_else(|| "大模型未返回内容".to_string())
}

fn truncate(s: &str) -> String {
    if s.chars().count() > 300 {
        s.chars().take(300).collect()
    } else {
        s.to_string()
    }
}
