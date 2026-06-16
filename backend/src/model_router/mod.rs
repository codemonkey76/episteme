use anyhow::Result;
use futures::StreamExt;
use genai::adapter::AdapterKind;
use genai::chat::{
    ChatMessage as GenaiMessage, ChatOptions, ChatRequest, ChatResponseFormat, ContentPart,
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
    /// Token counts when the provider reports them (on the done chunk).
    pub usage: Option<TokenUsage>,
    /// Reasoning ("thinking") deltas from a thinking model. Carried separately
    /// from `text` so the agent can drive a "thinking…" indicator without the
    /// trace leaking into the visible reply.
    pub reasoning: Option<String>,
}

/// Provider-reported token counts for one request — fed into the usage table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: i64,
    pub completion: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: Value,
}

/// Stateless façade over the provider adapters — providers are resolved from
/// settings at each call site, so there's nothing to construct or hold.
pub struct ModelRouter;

impl ModelRouter {
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
            return stream_ollama(provider, history, tools, think, false, tx).await;
        }

        // Non-Ollama providers use genai's NON-streaming exec_chat. genai 0.1.23's
        // streaming (exec_chat_stream) silently drops tool calls — captured_content
        // is only ever text — and its reqwest-eventsource can't clone Anthropic
        // requests. So the whole response (text and/or tool calls) is delivered at
        // once: one text chunk, then a done chunk carrying the tool calls. Cloud
        // replies therefore aren't token-streamed, but the agent's tools work.
        let client = build_client(provider);
        let genai_tools = schemas_to_tools(tools);
        let mut chat_req = ChatRequest::new(history_to_genai(history));
        if !genai_tools.is_empty() {
            chat_req = chat_req.with_tools(genai_tools);
        }
        let options = ChatOptions::default().with_capture_usage(true);
        let res = client.exec_chat(&provider.model_id, chat_req, Some(&options)).await?;

        let usage = Some(TokenUsage {
            prompt: res.usage.prompt_tokens.unwrap_or(0) as i64,
            completion: res.usage.completion_tokens.unwrap_or(0) as i64,
        });
        let text = res.content_text_as_str().map(str::to_string);
        let tool_calls = res.into_tool_calls();

        if let Some(text) = text.filter(|t| !t.is_empty()) {
            tx.send(StreamChunk { text, done: false, tool_calls: None, usage: None, reasoning: None })
                .await?;
        }
        tx.send(StreamChunk { text: String::new(), done: true, tool_calls, usage, reasoning: None })
            .await?;
        Ok(())
    }

    /// Run a completion to the end and return the full text plus provider-
    /// reported token counts (when given), discarding tool calls. For one-shot
    /// uses (e.g. JSON classification, memory consolidation) where streaming to a
    /// client isn't needed.
    pub async fn complete_with_usage(
        provider: &ProviderConfig,
        history: Vec<ChatMessage>,
    ) -> Result<(String, Option<TokenUsage>)> {
        Self::complete_inner(provider, history, false).await
    }

    /// Like `complete_with_usage`, but asks the provider for strict JSON. For
    /// Ollama this sets the native `format: "json"` constraint — decisive for
    /// weak local models that otherwise wrap the answer in prose (a JSON-array
    /// classifier prompt then comes back as narration, parsing to nothing).
    /// Cloud providers already honor the prompt's JSON instruction, so they go
    /// through the same path unchanged.
    pub async fn complete_json_with_usage(
        provider: &ProviderConfig,
        history: Vec<ChatMessage>,
    ) -> Result<(String, Option<TokenUsage>)> {
        Self::complete_inner(provider, history, true).await
    }

    async fn complete_inner(
        provider: &ProviderConfig,
        history: Vec<ChatMessage>,
        json: bool,
    ) -> Result<(String, Option<TokenUsage>)> {
        // Non-Ollama providers go through genai's NON-streaming `exec_chat`.
        // Streaming there uses reqwest-eventsource, which clones the request for
        // retries and fails on Anthropic with `CannotCloneRequestError`; a
        // one-shot completion has no reason to stream anyway.
        if provider.provider != "ollama" {
            let client = build_client(provider);
            let chat_req = ChatRequest::new(history_to_genai(history));
            let mut options = ChatOptions::default().with_capture_usage(true);
            // OpenAI-family adapters (incl. local models served over an
            // OpenAI-compatible endpoint) honor a json response_format. The
            // prompt already contains "JSON" (required by JsonMode). Anthropic/
            // Gemini/Cohere have no such mode and already comply, so leave them.
            if json && matches!(provider.provider.as_str(),
                "openai" | "openai_compatible" | "groq" | "xai" | "deepseek")
            {
                options = options.with_response_format(ChatResponseFormat::JsonMode);
            }
            let res = client.exec_chat(&provider.model_id, chat_req, Some(&options)).await?;
            let usage = Some(TokenUsage {
                prompt: res.usage.prompt_tokens.unwrap_or(0) as i64,
                completion: res.usage.completion_tokens.unwrap_or(0) as i64,
            });
            return Ok((res.content_text_into_string().unwrap_or_default(), usage));
        }

        // Ollama: collect from its custom streaming path (`json` sets format mode).
        let (tx, mut rx) = mpsc::channel::<StreamChunk>(64);
        let provider = provider.clone();
        let task = tokio::spawn(async move {
            stream_ollama(&provider, history, Vec::new(), false, json, tx).await
        });

        let mut out = String::new();
        let mut usage = None;
        while let Some(chunk) = rx.recv().await {
            out.push_str(&chunk.text);
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
        }
        // Surface a model/transport error rather than silently returning a partial string.
        task.await??;
        Ok((out, usage))
    }
}

async fn stream_ollama(
    provider: &ProviderConfig,
    history: Vec<ChatMessage>,
    tools: Vec<Value>,
    think: bool,
    json: bool,
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
            // The assistant's own tool calls — replayed so the model knows it
            // already acted (omitting these makes models repeat the call).
            if m.role == "tool_call" {
                let tool_calls: Vec<Value> = m
                    .content
                    .as_array()
                    .map(|calls| {
                        calls
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "function": {
                                        "name": c["fn_name"],
                                        "arguments": c["fn_arguments"],
                                    }
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                return serde_json::json!({ "role": "assistant", "content": "", "tool_calls": tool_calls });
            }
            // A tool result ({call_id, name?, content}) → Ollama's role:"tool".
            if m.role == "tool" {
                let content = match &m.content["content"] {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                let mut msg = serde_json::json!({ "role": "tool", "content": content });
                if let Some(name) = m.content["name"].as_str() {
                    msg["tool_name"] = Value::String(name.to_string());
                }
                return msg;
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
    // Constrain decoding to valid JSON. Decisive for one-shot JSON callers (the
    // email categorizer, etc.) on weaker local models, which otherwise lead with
    // prose — leaving no JSON to parse no matter how firmly the prompt asks.
    if json {
        body["format"] = serde_json::json!("json");
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
    // Ollama defaults num_ctx to 4096 and silently truncates bigger prompts
    // from the FRONT — the system prompt (instructions, JSON schemas) vanishes
    // first. Raise the window to fit when the prompt is large; small requests
    // keep the default so they don't pay the extra VRAM.
    if let Some(num_ctx) = ollama_num_ctx(body.to_string().len(), think) {
        // Preserve any options already set above (none today, but keep this
        // robust to future additions) rather than clobbering the object.
        body["options"]["num_ctx"] = serde_json::json!(num_ctx);
    }

    // A read (not total) timeout: streaming replies can run for minutes on a
    // slow local model, but a genuinely wedged Ollama (or a dropped socket
    // that never errors) stops sending bytes — without this the request, and
    // any job awaiting it, hangs forever. read_timeout fires only on an idle
    // gap between bytes, so legitimate slow streams are unaffected.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| anyhow::anyhow!("Ollama client build error: {e}"))?;
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
                    tx.send(StreamChunk {
                        text: content.to_string(),
                        done: false,
                        tool_calls: None,
                        usage: None,
                        reasoning: None,
                    })
                    .await?;
                }
            }

            // Reasoning deltas (thinking models): forwarded as `reasoning` so the
            // agent can show a "thinking…" indicator. Never folded into `text` —
            // the trace isn't the answer.
            if let Some(thinking) = data["message"]["thinking"].as_str() {
                if !thinking.is_empty() {
                    tx.send(StreamChunk {
                        text: String::new(),
                        done: false,
                        tool_calls: None,
                        usage: None,
                        reasoning: Some(thinking.to_string()),
                    })
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
                // `done_reason == "length"` means the reply hit the context
                // window before finishing — the visible answer can come back
                // empty (the budget was spent on the prompt and/or thinking
                // trace). Surface it so a silent empty turn isn't a mystery.
                if data["done_reason"].as_str() == Some("length") {
                    tracing::warn!(
                        "Ollama reply truncated (done_reason=length, num_ctx exhausted) for model {}",
                        provider.model_id
                    );
                }
                // Ollama reports token counts on the final message.
                let usage = match (data["prompt_eval_count"].as_i64(), data["eval_count"].as_i64()) {
                    (None, None) => None,
                    (p, c) => Some(TokenUsage {
                        prompt: p.unwrap_or(0),
                        completion: c.unwrap_or(0),
                    }),
                };
                tx.send(StreamChunk { text: String::new(), done: true, tool_calls, usage, reasoning: None })
                    .await?;
                return Ok(());
            }
        }
    }

    tx.send(StreamChunk { text: String::new(), done: true, tool_calls: None, usage: None, reasoning: None })
        .await?;
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

            // Treat empty strings as "not set": the settings form stores absent
            // fields as "" (not null), and an empty base_url would otherwise
            // override the adapter's default endpoint with a blank URL
            // (RelativeUrlWithoutBase), and an empty key would blank the auth.
            match api_key.as_deref().filter(|k| !k.is_empty()) {
                Some(key) => target.auth = AuthData::from_single(key.to_string()),
                None => {
                    if let Some(env_name) = adapter_kind.default_key_env_name() {
                        target.auth = AuthData::from_env(env_name);
                    }
                }
            }

            if let Some(url) = base_url.as_deref().filter(|u| !u.is_empty()) {
                target.endpoint = Endpoint::from_owned(url.to_string());
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
            // The assistant's own tool calls — replayed so the model knows it
            // already acted (omitting these makes models repeat the call).
            if m.role == "tool_call" {
                let calls: Vec<genai::chat::ToolCall> =
                    serde_json::from_value(m.content).unwrap_or_default();
                if calls.is_empty() {
                    return None;
                }
                return Some(GenaiMessage::from(calls));
            }
            // A tool result ({call_id, name?, content}).
            if m.role == "tool" {
                let call_id = m.content["call_id"].as_str().unwrap_or_default().to_string();
                let content = match &m.content["content"] {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                return Some(GenaiMessage::from(genai::chat::ToolResponse::new(call_id, content)));
            }
            let text = match m.content {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            match m.role.as_str() {
                "user" => Some(GenaiMessage::user(text)),
                "assistant" => Some(GenaiMessage::assistant(text)),
                "system" => Some(GenaiMessage::system(text)),
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

/// Context window override for an Ollama request, from the serialized request
/// size. None = the prompt fits Ollama's 4096 default; otherwise the needed
/// window rounded up in 4k steps, capped at `CAP` (Ollama clamps to the model's
/// own maximum beyond that). Bytes/3 deliberately over-estimates tokens
/// (English runs ~4 bytes/token) so truncation stays the rare case.
///
/// `think` reserves far more reply headroom: a reasoning model emits a hidden
/// thinking trace BEFORE any visible content, often several thousand tokens. At
/// the old 2k headroom a big prompt left no room for the trace + answer, so the
/// window filled mid-think and the reply came back empty (done_reason=length) —
/// a silent, blank turn. 12k headroom keeps the visible answer inside the window.
///
/// CAP is 128k: the hybrid-SSM Qwen3.x models advertise 256k and grow KV memory
/// only gently with context, so 128k loads in ~26 GB (fits a 24 GB GPU with a
/// small CPU spill). The window is still sized to the prompt — small chats stay
/// cheap; only a genuinely large transcript asks for the full 128k.
fn ollama_num_ctx(request_bytes: usize, think: bool) -> Option<usize> {
    const DEFAULT: usize = 4096;
    const CAP: usize = 131_072;
    let reply_headroom = if think { 12_288 } else { 2_048 };
    let needed = request_bytes / 3 + reply_headroom;
    (needed > DEFAULT).then(|| needed.next_multiple_of(4096).min(CAP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_num_ctx_scales_with_prompt_size() {
        // Small prompts keep Ollama's default window (no options sent).
        assert_eq!(ollama_num_ctx(1_000, false), None);
        assert_eq!(ollama_num_ctx(6_000, false), None);
        // A deep-research synthesis prompt (~30k bytes) needs ~12k tokens.
        assert_eq!(ollama_num_ctx(30_000, false), Some(12_288));
        // Huge prompts cap at 128k.
        assert_eq!(ollama_num_ctx(500_000, false), Some(131_072));
    }

    #[test]
    fn ollama_num_ctx_reserves_headroom_for_thinking() {
        // Thinking turns reserve 12k tokens for the (hidden) trace + answer, so
        // the same prompt asks for a much larger window than a non-thinking one.
        assert_eq!(ollama_num_ctx(30_000, true), Some(24_576));
        // The 12k headroom alone clears the 4096 default, so even a tiny prompt
        // now gets an explicit window (333 + 12288 -> 16384).
        assert_eq!(ollama_num_ctx(1_000, true), Some(16_384));
        // Big prompts cap at 128k just the same.
        assert_eq!(ollama_num_ctx(500_000, true), Some(131_072));
    }
}
