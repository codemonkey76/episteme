use axum::{extract::State, Json};
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::model_router::ProviderConfig;
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

/// Decoded audio cap — a few minutes of compressed speech.
const AUDIO_MAX: usize = 15 * 1024 * 1024;

#[derive(Deserialize)]
pub struct TranscribeBody {
    /// Raw audio bytes, base64-encoded (no `data:` prefix).
    audio_b64: String,
    /// e.g. audio/m4a, audio/webm — used for the upload filename hint.
    #[serde(default)]
    mime: Option<String>,
}

/// Whisper endpoint + model for a provider that can transcribe. Groq is
/// preferred (fast, near-free); OpenAI works identically.
fn whisper_target(p: &ProviderConfig) -> Option<(String, &'static str, String)> {
    let (url, model, key_env) = match p.provider.as_str() {
        "groq" => (
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-v3-turbo",
            "GROQ_API_KEY",
        ),
        "openai" => (
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
            "OPENAI_API_KEY",
        ),
        _ => return None,
    };
    let key = p
        .api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .or_else(|| std::env::var(key_env).ok())?;
    Some((url.to_string(), model, key))
}

// POST /api/transcribe — speech-to-text via the first Groq (or OpenAI)
// provider's Whisper endpoint. Returns { text }.
pub async fn transcribe(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(_user)): Extension<CurrentUser>,
    Json(body): Json<TranscribeBody>,
) -> AppResult<Json<Value>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.audio_b64.trim())
        .map_err(|e| AppError::BadRequest(format!("audio is not valid base64: {e}")))?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("audio is empty".into()));
    }
    if bytes.len() > AUDIO_MAX {
        return Err(AppError::BadRequest("audio exceeds the 15 MB limit".into()));
    }

    let providers: Vec<ProviderConfig> = db::settings::get(&state.db, "providers")
        .await
        .map_err(AppError::Internal)?
        .unwrap_or_default();
    // Groq first, OpenAI as fallback — both expose the same Whisper API shape.
    let (url, model, key) = providers
        .iter()
        .filter(|p| p.provider == "groq")
        .chain(providers.iter().filter(|p| p.provider == "openai"))
        .find_map(whisper_target)
        .ok_or_else(|| {
            AppError::BadRequest(
                "voice transcription needs a Groq (or OpenAI) provider with an API key".into(),
            )
        })?;

    let mime = body.mime.unwrap_or_else(|| "audio/m4a".to_string());
    let ext = mime.rsplit('/').next().unwrap_or("m4a").to_string();
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(format!("voice.{ext}"))
        .mime_str(&mime)
        .map_err(|e| AppError::BadRequest(format!("invalid mime type: {e}")))?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model)
        .text("response_format", "json");

    let response = state
        .http_client
        .post(&url)
        .bearer_auth(&key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("transcription request failed: {e}")))?;

    let status = response.status();
    let parsed: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = parsed["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(AppError::Internal(anyhow::anyhow!("transcription failed: {status} {msg}")));
    }

    let text = parsed["text"].as_str().unwrap_or_default().trim().to_string();
    Ok(Json(serde_json::json!({ "text": text })))
}
