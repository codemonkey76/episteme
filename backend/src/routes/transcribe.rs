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

/// Where the local Whisper sidecar lives (compose service `whisper`).
/// Env-only on purpose: it's compose topology, not a user setting.
fn whisper_url() -> String {
    std::env::var("WHISPER_URL").unwrap_or_else(|_| "http://whisper:8000".to_string())
}

/// Model the sidecar should run; downloads from HF on first use.
fn whisper_model() -> String {
    std::env::var("WHISPER_MODEL").unwrap_or_else(|_| "Systran/faster-whisper-small".to_string())
}

/// One place audio can be transcribed: an OpenAI-shaped endpoint, its model,
/// and a bearer key for the hosted ones (the local sidecar is unauthed).
struct WhisperTarget {
    url: String,
    model: String,
    key: Option<String>,
}

/// Whisper endpoint + model for a provider that can transcribe. Groq is
/// preferred (fast, near-free); OpenAI works identically.
fn whisper_target(p: &ProviderConfig) -> Option<WhisperTarget> {
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
    Some(WhisperTarget { url: url.to_string(), model: model.to_string(), key: Some(key) })
}

/// Everywhere we can send this audio, in preference order: the self-hosted
/// sidecar first (private, free), then Groq, then OpenAI. Later targets are
/// fallbacks when an earlier one is down or errors.
fn transcription_targets(providers: &[ProviderConfig]) -> Vec<WhisperTarget> {
    let mut targets = vec![WhisperTarget {
        url: format!("{}/v1/audio/transcriptions", whisper_url().trim_end_matches('/')),
        model: whisper_model(),
        key: None,
    }];
    targets.extend(
        providers
            .iter()
            .filter(|p| p.provider == "groq")
            .chain(providers.iter().filter(|p| p.provider == "openai"))
            .filter_map(whisper_target),
    );
    targets
}

// POST /api/transcribe — speech-to-text via the local Whisper sidecar,
// falling back to a configured Groq/OpenAI provider. Returns { text }.
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
    let mime = body.mime.unwrap_or_else(|| "audio/m4a".to_string());
    let ext = mime.rsplit('/').next().unwrap_or("m4a").to_string();

    // Sidecar first, hosted Whisper as fallback; remember why each leg failed.
    let mut failures: Vec<String> = Vec::new();
    for target in transcription_targets(&providers) {
        // One install-and-retry per target: the sidecar doesn't pull models on
        // demand, so a fresh volume answers "not installed" until we ask it to.
        let mut installed = false;
        loop {
            let part = reqwest::multipart::Part::bytes(bytes.clone())
                .file_name(format!("voice.{ext}"))
                .mime_str(&mime)
                .map_err(|e| AppError::BadRequest(format!("invalid mime type: {e}")))?;
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("model", target.model.clone())
                .text("response_format", "json");

            let mut request = state.http_client.post(&target.url).multipart(form);
            if let Some(key) = &target.key {
                request = request.bearer_auth(key);
            }
            let response = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{}: {e}", target.url));
                    break;
                }
            };

            let status = response.status();
            let parsed: Value = response.json().await.unwrap_or(Value::Null);
            if status.is_success() {
                let text = parsed["text"].as_str().unwrap_or_default().trim().to_string();
                return Ok(Json(serde_json::json!({ "text": text })));
            }
            // Hosted endpoints put errors in error.message; speaches in detail.
            let msg = parsed["error"]["message"]
                .as_str()
                .or_else(|| parsed["detail"].as_str())
                .unwrap_or("unknown error");
            if target.key.is_none() && !installed && msg.contains("not installed") {
                installed = true;
                if let Some(base) = target.url.strip_suffix("/audio/transcriptions") {
                    let pull = state
                        .http_client
                        .post(format!("{base}/models/{}", target.model))
                        .send()
                        .await;
                    if pull.is_ok_and(|r| r.status().is_success()) {
                        continue;
                    }
                }
            }
            failures.push(format!("{}: {status} {msg}", target.url));
            break;
        }
    }

    Err(AppError::Internal(anyhow::anyhow!(
        "transcription failed — no Whisper endpoint reachable (is the `whisper` compose service \
         running, or a Groq/OpenAI provider configured?): {}",
        failures.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: &str, key: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            name: kind.to_string(),
            provider: kind.to_string(),
            base_url: None,
            api_key: key.map(str::to_string),
            model_id: "x".to_string(),
        }
    }

    #[test]
    fn targets_lead_with_the_local_sidecar() {
        let targets = transcription_targets(&[]);
        assert_eq!(targets.len(), 1);
        assert!(targets[0].url.ends_with("/v1/audio/transcriptions"));
        assert!(targets[0].key.is_none());
    }

    #[test]
    fn hosted_targets_follow_groq_before_openai() {
        let providers = vec![
            provider("openai", Some("sk-1")),
            provider("ollama", None),
            provider("groq", Some("gsk-1")),
        ];
        let targets = transcription_targets(&providers);
        assert_eq!(targets.len(), 3);
        assert!(targets[0].key.is_none()); // local sidecar
        assert!(targets[1].url.contains("groq.com"));
        assert!(targets[2].url.contains("openai.com"));
    }

    #[test]
    fn keyless_hosted_providers_are_skipped() {
        // (Assumes GROQ_API_KEY is unset in the test environment.)
        if std::env::var("GROQ_API_KEY").is_ok() {
            return;
        }
        let targets = transcription_targets(&[provider("groq", None)]);
        assert_eq!(targets.len(), 1);
    }
}
