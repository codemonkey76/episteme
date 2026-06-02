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
        &self,
        provider_name: &str,
        history: Vec<ChatMessage>,
        tools: Vec<Value>,
        tx: mpsc::Sender<StreamChunk>,
    ) -> Result<()> {
        let provider = self
            .get_provider(provider_name)
            .ok_or_else(|| anyhow::anyhow!("provider '{provider_name}' not found"))?
            .clone();

        let client = build_client(&provider);

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
