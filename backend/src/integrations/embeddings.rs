//! Text embeddings via the user's Ollama instance — the engine behind
//! semantic memory (and, later, document RAG). Deliberately Ollama-only:
//! memory content never leaves the machine for a cloud embedding API.
//!
//! Vectors are stored as little-endian f32 BLOBs in SQLite and compared with
//! brute-force cosine in Rust — plenty fast into the tens of thousands of
//! rows, with no native-extension dependency.

use anyhow::{anyhow, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::model_router::ProviderConfig;

const DEFAULT_MODEL: &str = "nomic-embed-text";
const DEFAULT_OLLAMA: &str = "http://localhost:11434";

/// Global cap on concurrent embed requests in flight to Ollama. The embed model
/// shares one GPU with the (much larger) chat model, so a burst of parallel
/// requests — email indexing + memory backfill firing at once — used to flood
/// Ollama's scheduler queue and get rejected en masse with
/// "503 server busy, maximum pending requests exceeded". Serialising to a small
/// number keeps us well under that ceiling; embeddings are best-effort
/// background work, so trickling them through is fine.
static EMBED_GATE: Semaphore = Semaphore::const_new(2);

/// How many times to retry a transient "server busy" 503 before giving up.
const BUSY_RETRIES: u32 = 5;

/// The embedding model, overridable via the `embedding_model` settings key.
async fn model(pool: &SqlitePool) -> String {
    crate::db::settings::get::<String>(pool, "embedding_model")
        .await
        .ok()
        .flatten()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// Base URL of the first configured Ollama provider (they all point at the
/// same instance in practice), falling back to localhost.
async fn ollama_base(pool: &SqlitePool) -> String {
    let providers: Vec<ProviderConfig> =
        crate::db::settings::get(pool, "providers").await.ok().flatten().unwrap_or_default();
    providers
        .into_iter()
        .find(|p| p.provider == "ollama")
        .and_then(|p| p.base_url)
        .unwrap_or_else(|| DEFAULT_OLLAMA.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Embed one text. Errors (Ollama down, model not pulled) bubble up — callers
/// treat embeddings as best-effort and fall back to recency. Takes the pool +
/// client (both cheap clones) rather than `AppState` so detached tasks can own
/// their captures.
pub async fn embed(pool: &SqlitePool, client: &reqwest::Client, text: &str) -> Result<Vec<f32>> {
    let base = ollama_base(pool).await;
    let model = model(pool).await;

    // Hold a permit for the whole request so total in-flight embeds stay capped
    // (see EMBED_GATE). const_new() never closes, so acquire can't fail.
    let _permit = EMBED_GATE.acquire().await.expect("embed semaphore");

    // Ollama returns a 503 "server busy" when its scheduler queue is saturated;
    // that's transient, so back off and retry rather than dropping the work.
    let mut attempt = 0;
    let body: Value = loop {
        let response = client
            .post(format!("{base}/api/embeddings"))
            .json(&serde_json::json!({ "model": model, "prompt": text }))
            .send()
            .await
            .map_err(|e| anyhow!("embedding request failed: {e}"))?;

        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            break body;
        }

        let msg = body["error"].as_str().unwrap_or("unknown error");
        let busy = status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
        if busy && attempt < BUSY_RETRIES {
            // Exponential backoff: 250ms, 500ms, 1s, 2s, 4s.
            let delay = std::time::Duration::from_millis(250 << attempt);
            tokio::time::sleep(delay).await;
            attempt += 1;
            continue;
        }
        return Err(anyhow!(
            "embedding failed: {status} {msg} (is `{model}` pulled on the Ollama host?)"
        ));
    };

    let vec: Vec<f32> = body["embedding"]
        .as_array()
        .ok_or_else(|| anyhow!("embedding response missing vector"))?
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();
    if vec.is_empty() {
        return Err(anyhow!("embedding response empty"));
    }
    Ok(vec)
}

/// f32 slice → little-endian byte blob for SQLite storage.
pub fn to_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Little-endian byte blob → f32 vector. Truncated/odd blobs yield what fits.
pub fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Cosine similarity; 0.0 for mismatched lengths or zero vectors, so bad data
/// just ranks last instead of erroring.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip() {
        let v = vec![0.5f32, -1.25, 3.0, f32::MIN_POSITIVE];
        assert_eq!(from_blob(&to_blob(&v)), v);
        assert!(from_blob(&[1, 2, 3]).is_empty()); // truncated blob
    }

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_degenerate_inputs_rank_last() {
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0); // length mismatch
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 2.0]), 0.0); // zero vector
        assert_eq!(cosine(&[], &[]), 0.0);
    }
}
