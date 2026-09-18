//! 大模型调用：听写文本润色、命令通道的意图路由，以及 API Key 可用性验证。

use crate::config::PolishMode;
use serde::Deserialize;

/// 百炼 OpenAI 兼容端点，旧域名仍可正常使用
const API_BASE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";

/// 所有请求共用：整体 30 秒超时，防止润色或命令请求无限挂起卡住字幕条
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

const POLISH_SYSTEM: &str = "\
你是语音转写校对员，不是改写者。用户的文本来自语音识别，你只修复转写与口语本身的瑕疵，严格遵守最小修改原则。\n\
\n\
只做以下五类修正：\n\
1. 删除纯填充的语气词（嗯、啊、呃、这个、那个、就是说等）和无意义的重复字词；有实义的要保留，比如“那个文件”里的“那个”\n\
2. 删除说一半就改口的残句碎片，保留改口后的完整说法\n\
3. 修正明显的同音错别字与识别错误，拿不准就保留原文\n\
4. 按语气整理标点和断句\n\
5. 修正明显听错的专有名词\n\
\n\
铁律：\n\
- 输出保持说话人原有的用词、语序、句式和口吻，仍是口语，禁止书面化\n\
- 禁止增删或改动任何实质内容，禁止同义替换、概括、扩写\n\
- 禁止回答、评论或输出任何解释、前缀、后缀\n\
- 输入为空或没有实际内容时，只输出空字符串\n\
\n\
示例：\n\
输入：嗯那个我们明天下午三点半，那个，开一下周会对吧，呃没空的话说一声\n\
输出：我们明天下午三点半开一下周会对吧，没空的话说一声\n\
输入：把这个文件啊同步到，同步到网盘上面去，今天之内\n\
输出：把这个文件同步到网盘上面去，今天之内";

/// 把识别原文润色为书面语
pub async fn polish(key: &str, model: &str, text: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        // 保真优先：低温抑制模型自由发挥
        "temperature": 0.1,
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

    let client = client();
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
        return Err(format!("大模型返回错误 {status}: {}", truncate(&body)));
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

/// 拉取账号可用的模型列表，只保留 qwen 系列
pub async fn list_models(key: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    #[derive(Deserialize)]
    struct Resp {
        data: Vec<Model>,
    }

    let client = client();
    let resp = client
        .get(format!("{API_BASE}/models"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .map_err(|e| format!("获取模型列表失败: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("获取模型列表失败 {status}: {}", truncate(&body)));
    }

    let parsed: Resp = resp
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败: {e}"))?;
    Ok(filter_qwen_models(parsed.data.into_iter().map(|m| m.id)))
}

/// 从模型 id 里筛出 qwen 开头的系列，去重并按字母序排列
fn filter_qwen_models(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut models: Vec<String> = ids.into_iter().filter(|id| id.starts_with("qwen")).collect();
    models.sort();
    models.dedup();
    models
}

async fn chat(key: &str, body: serde_json::Value) -> Result<String, String> {
    let client = client();
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
        return Err(format!("大模型返回错误 {status}: {}", truncate(&body)));
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

#[cfg(test)]
mod tests {
    use super::filter_qwen_models;

    #[test]
    fn filters_qwen_and_sorts() {
        let ids = vec![
            "fun-asr-realtime".to_string(),
            "qwen-max".to_string(),
            "qwen3.7-flash".to_string(),
            "deepseek-v3".to_string(),
            "qwen-max".to_string(),
        ];
        assert_eq!(
            filter_qwen_models(ids),
            vec!["qwen-max".to_string(), "qwen3.7-flash".to_string()]
        );
    }

    #[test]
    fn empty_when_no_qwen() {
        assert!(filter_qwen_models(vec!["paraformer-v2".to_string()]).is_empty());
    }
}
