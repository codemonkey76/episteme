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

use crate::model_router::ProviderConfig;

const DEFAULT_MODEL: &str = "nomic-embed-text";
const DEFAULT_OLLAMA: &str = "http://localhost:11434";

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
    let response = client
        .post(format!("{base}/api/embeddings"))
        .json(&serde_json::json!({ "model": model, "prompt": text }))
        .send()
        .await
        .map_err(|e| anyhow!("embedding request failed: {e}"))?;

    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["error"].as_str().unwrap_or("unknown error");
        return Err(anyhow!(
            "embedding failed: {status} {msg} (is `{model}` pulled on the Ollama host?)"
        ));
    }

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
