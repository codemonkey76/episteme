use anyhow::Result;
use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{
    ChatMessage as GenaiMessage, ChatOptions, ChatRequest, ChatStreamEvent, ContentPart,
    MessageContent, Tool,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    /// One of: anthropic, openai, openai_compatible, ollama, gemini, cohere, groq, xai, deepseek
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_id: String,
}

#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub text: String,
    pub done: bool,
    /// Non-empty when the model requested a tool call (captured at End event).
    pub tool_calls: Option<Vec<genai::chat::ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

pub struct ModelRouter {
    providers: Vec<ProviderConfig>,
}

impl ModelRouter {
    pub fn new() -> Self {
        Self { providers: Vec::new() }
    }

    pub fn set_providers(&mut self, providers: Vec<ProviderConfig>) {
        self.providers = providers;
    }

    pub fn providers(&self) -> &[ProviderConfig] {
        &self.providers
    }

    pub fn get_provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Stream a completion for the given provider. Emits text `StreamChunk`s and a final
    /// done chunk that may carry tool calls if the model requested them.
    pub async fn stream(
        provider: &ProviderConfig,
        history: Vec<ChatMessage>,
        tools: Vec<Value>,
        think: bool,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<()> {
        if provider.provider == "ollama" {
            return stream_ollama(provider, history, tools, think, tx).await;
        }

        let client = build_client(provider);

        let genai_messages = history_to_genai(history);
        let genai_tools = schemas_to_tools(tools);

        let mut chat_req = ChatRequest::new(genai_messages);
        if !genai_tools.is_empty() {
            chat_req = chat_req.with_tools(genai_tools);
        }

        let options = ChatOptions::default().with_capture_content(true);

        let mut stream = client
            .exec_chat_stream(&provider.model_id, chat_req, Some(&options))
            .await?
            .stream;

        while let Some(event) = stream.next().await {
            match event? {
                ChatStreamEvent::Start | ChatStreamEvent::ReasoningChunk(_) => {}
                ChatStreamEvent::Chunk(chunk) => {
                    tx.send(StreamChunk { text: chunk.content, done: false, tool_calls: None })
                        .await?;
                }
                ChatStreamEvent::End(end) => {
                    let tool_calls = end.captured_content.and_then(|c| match c {
                        genai::chat::MessageContent::ToolCalls(calls) => Some(calls),
                        _ => None,
                    });
                    tx.send(StreamChunk { text: String::new(), done: true, tool_calls }).await?;
                }
            }
        }

        Ok(())
    }

    /// Run a completion to the end and return the full text, discarding tool
    /// calls. For one-shot uses (e.g. JSON classification) where streaming the
    /// tokens to a client isn't needed.
    pub async fn complete(provider: &ProviderConfig, history: Vec<ChatMessage>) -> Result<String> {
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let provider = provider.clone();
        let task = tokio::spawn(async move {
            Self::stream(&provider, history, Vec::new(), false, tx).await
        });

        let mut out = String::new();
        while let Some(chunk) = rx.recv().await {
            out.push_str(&chunk.text);
        }
        // Surface a model/transport error rather than silently returning a partial string.
        task.await??;
        Ok(out)
    }
}

async fn stream_ollama(
    provider: &ProviderConfig,
    history: Vec<ChatMessage>,
    tools: Vec<Value>,
    think: bool,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<()> {
    let base_url = provider.base_url.as_deref().unwrap_or("http://localhost:11434");
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let messages: Vec<Value> = history
        .into_iter()
        .map(|m| {
            // Multimodal user message → Ollama's {content, images:[b64,...]} form.
            if m.role == "user" && m.content.get("type").and_then(Value::as_str) == Some("multimodal") {
                let text = m.content.get("text").and_then(Value::as_str).unwrap_or_default();
                let images: Vec<Value> = m
                    .content
                    .get("images")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|img| img.get("b64").and_then(Value::as_str))
                            .map(|s| Value::String(s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                return serde_json::json!({ "role": "user", "content": text, "images": images });
            }
            let content = match m.content {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            serde_json::json!({ "role": m.role, "content": content })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": provider.model_id,
        "messages": messages,
        "stream": true,
        // Keep the model resident after a request so the next chat/draft doesn't
        // pay a full cold-load (Ollama unloads after 5 min idle by default).
        "keep_alive": "30m",
    });
    // Disable reasoning for thinking-capable models (e.g. qwen3, deepseek-r1) so
    // they emit the answer immediately. Only sent when explicitly disabling —
    // omitted otherwise to preserve default behavior for non-thinking models.
    if !think {
        body["think"] = serde_json::json!(false);
    }
    // Advertise tools in Ollama's format ({type, function:{name,description,parameters}}).
    // Without this the model can't call tools and tends to hallucinate that it did.
    let ollama_tools: Vec<Value> = tools
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?;
            Some(serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "parameters": t.get("input_schema").cloned().unwrap_or(serde_json::json!({ "type": "object" })),
                }
            }))
        })
        .collect();
    if !ollama_tools.is_empty() {
        body["tools"] = serde_json::json!(ollama_tools);
    }

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Ollama connection error: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let err: Value = response.json().await.unwrap_or(Value::Null);
        anyhow::bail!("Ollama {status}: {}", err["error"].as_str().unwrap_or("unknown error"));
    }

    let mut byte_stream = response.bytes_stream();
    let mut buf = String::new();
    // Tool calls may arrive across chunks; accumulate and emit on the done chunk.
    let mut pending_tool_calls: Vec<genai::chat::ToolCall> = Vec::new();

    while let Some(chunk) = byte_stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk?));

        while let Some(newline) = buf.find('\n') {
            let line = buf[..newline].trim().to_string();
            buf = buf[newline + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            let Ok(data) = serde_json::from_str::<Value>(&line) else {
                continue;
            };

            if let Some(content) = data["message"]["content"].as_str() {
                if !content.is_empty() {
                    tx.send(StreamChunk { text: content.to_string(), done: false, tool_calls: None })
                        .await?;
                }
            }

            // Collect any tool calls the model requested.
            if let Some(calls) = data["message"]["tool_calls"].as_array() {
                for tc in calls {
                    let func = &tc["function"];
                    if let Some(name) = func["name"].as_str() {
                        pending_tool_calls.push(genai::chat::ToolCall {
                            call_id: format!("ollama_call_{}", pending_tool_calls.len()),
                            fn_name: name.to_string(),
                            fn_arguments: func["arguments"].clone(),
                        });
                    }
                }
            }

            if data["done"].as_bool().unwrap_or(false) {
                let tool_calls = if pending_tool_calls.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut pending_tool_calls))
                };
                tx.send(StreamChunk { text: String::new(), done: true, tool_calls }).await?;
                return Ok(());
            }
        }
    }

    tx.send(StreamChunk { text: String::new(), done: true, tool_calls: None }).await?;
    Ok(())
}

fn build_client(provider: &ProviderConfig) -> Client {
    let adapter_kind = provider_to_adapter_kind(&provider.provider);
    let model_id = provider.model_id.clone();
    let api_key = provider.api_key.clone();
    let base_url = provider.base_url.clone();

    let resolver = ServiceTargetResolver::from_resolver_fn(
        move |mut target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            target.model = ModelIden::new(adapter_kind, model_id.clone());

            if let Some(ref key) = api_key {
                target.auth = AuthData::from_single(key.clone());
            } else if let Some(env_name) = adapter_kind.default_key_env_name() {
                target.auth = AuthData::from_env(env_name);
            }

            if let Some(ref url) = base_url {
                target.endpoint = Endpoint::from_owned(url.as_str());
            }

            Ok(target)
        },
    );

    Client::builder().with_service_target_resolver(resolver).build()
}

fn provider_to_adapter_kind(provider: &str) -> AdapterKind {
    match provider {
        "anthropic" => AdapterKind::Anthropic,
        "openai" | "openai_compatible" => AdapterKind::OpenAI,
        "ollama" => AdapterKind::Ollama,
        "gemini" => AdapterKind::Gemini,
        "cohere" => AdapterKind::Cohere,
        "groq" => AdapterKind::Groq,
        "xai" => AdapterKind::Xai,
        "deepseek" => AdapterKind::DeepSeek,
        _ => AdapterKind::Ollama,
    }
}

fn history_to_genai(history: Vec<ChatMessage>) -> Vec<GenaiMessage> {
    history
        .into_iter()
        .filter_map(|m| {
            // A multimodal user message ({type:"multimodal", text, images:[...]})
            // becomes a parts message with text + base64 image parts.
            if m.role == "user" {
                if let Some(parts) = multimodal_parts(&m.content) {
                    return Some(GenaiMessage::user(MessageContent::from_parts(parts)));
                }
            }
            let text = match m.content {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            match m.role.as_str() {
                "user" => Some(GenaiMessage::user(text)),
                "assistant" => Some(GenaiMessage::assistant(text)),
                "system" => Some(GenaiMessage::system(text)),
                // Tool result messages are deferred until agent loop is fully wired.
                _ => None,
            }
        })
        .collect()
}

/// If `content` is a multimodal message object, build genai content parts
/// (text + base64 images). Returns None for plain-text content.
fn multimodal_parts(content: &Value) -> Option<Vec<ContentPart>> {
    if content.get("type").and_then(Value::as_str) != Some("multimodal") {
        return None;
    }
    let mut parts: Vec<ContentPart> = Vec::new();
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        if !text.is_empty() {
            parts.push(ContentPart::from_text(text.to_string()));
        }
    }
    if let Some(images) = content.get("images").and_then(Value::as_array) {
        for img in images {
            let mime = img.get("mime").and_then(Value::as_str).unwrap_or("image/png");
            if let Some(b64) = img.get("b64").and_then(Value::as_str) {
                parts.push(ContentPart::from_image_base64(mime.to_string(), b64.to_string()));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn schemas_to_tools(schemas: Vec<Value>) -> Vec<Tool> {
    schemas
        .into_iter()
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?.to_string();
            let description = v
                .get("description")
                .and_then(|d| d.as_str())
                .map(str::to_string);
            let schema = v.get("input_schema").cloned();

            let mut tool = Tool::new(name);
            if let Some(desc) = description {
                tool = tool.with_description(desc);
            }
            if let Some(s) = schema {
                tool = tool.with_schema(s);
            }
            Some(tool)
        })
        .collect()
}
