use anyhow::Result;
use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage as GenaiMessage, ChatOptions, ChatRequest, ChatStreamEvent, Tool};
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
            return stream_ollama(provider, history, think, tx).await;
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
}

async fn stream_ollama(
    provider: &ProviderConfig,
    history: Vec<ChatMessage>,
    think: bool,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<()> {
    let base_url = provider.base_url.as_deref().unwrap_or("http://localhost:11434");
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let messages: Vec<Value> = history
        .into_iter()
        .map(|m| {
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
    });
    // Disable reasoning for thinking-capable models (e.g. qwen3, deepseek-r1) so
    // they emit the answer immediately. Only sent when explicitly disabling —
    // omitted otherwise to preserve default behavior for non-thinking models.
    if !think {
        body["think"] = serde_json::json!(false);
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

            if data["done"].as_bool().unwrap_or(false) {
                tx.send(StreamChunk { text: String::new(), done: true, tool_calls: None }).await?;
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
