//! Document ingestion for RAG: extract text from an upload, split it into
//! overlapping chunks, and embed each chunk (detached) so `search_documents`
//! can retrieve by meaning. Storage lives in `db::documents`.

use anyhow::{anyhow, Result};
use sqlx::SqlitePool;

use crate::db;
use crate::integrations::embeddings;
use crate::state::AppState;

/// Target chunk size in characters (~300 tokens) and overlap between chunks.
const CHUNK_SIZE: usize = 1200;
const CHUNK_OVERLAP: usize = 200;
/// Refuse extractions beyond this many chars (~2.5 MB of text) — keeps a
/// pathological upload from generating tens of thousands of chunks.
const TEXT_MAX: usize = 2_500_000;

/// Extract plain text from an uploaded file by mime/extension.
pub fn extract_text(filename: &str, mime: &str, bytes: &[u8]) -> Result<String> {
    let ext = filename.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();

    let text = if mime == "application/pdf" || ext == "pdf" {
        pdf_extract::extract_text_from_mem(bytes)
            .map_err(|e| anyhow!("couldn't extract text from PDF: {e}"))?
    } else if mime.starts_with("text/html") || ext == "html" || ext == "htm" {
        crate::integrations::graph::html_to_text(&String::from_utf8_lossy(bytes))
    } else if mime.starts_with("text/")
        || matches!(ext.as_str(), "md" | "txt" | "csv" | "json" | "yaml" | "yml" | "toml" | "log")
        || mime == "application/json"
    {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        return Err(anyhow!(
            "unsupported file type '{mime}' — text, markdown, HTML, CSV, JSON, and PDF are supported"
        ));
    };

    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(anyhow!("no extractable text in '{filename}'"));
    }
    if text.chars().count() > TEXT_MAX {
        return Err(anyhow!("'{filename}' is too large to index (>2.5M characters of text)"));
    }
    Ok(text)
}

/// Split text into ~CHUNK_SIZE-char pieces with CHUNK_OVERLAP carry-over,
/// preferring paragraph then line then word boundaries near the target.
pub fn chunk(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < chars.len() {
        let hard_end = (start + CHUNK_SIZE).min(chars.len());
        let end = if hard_end == chars.len() {
            hard_end
        } else {
            // Search backwards from the hard end for a natural break, but
            // don't shrink the chunk below half size.
            let floor = start + CHUNK_SIZE / 2;
            find_break(&chars, floor, hard_end)
        };
        let piece: String = chars[start..end].iter().collect();
        let piece = piece.trim();
        if !piece.is_empty() {
            chunks.push(piece.to_string());
        }
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP).max(start + 1);
    }
    chunks
}

/// Best break position in (floor, end]: paragraph > newline > space > hard cut.
fn find_break(chars: &[char], floor: usize, end: usize) -> usize {
    let window = &chars[floor..end];
    // Paragraph break: "\n\n".
    if let Some(pos) = window
        .windows(2)
        .rposition(|w| w[0] == '\n' && w[1] == '\n')
    {
        return floor + pos + 2;
    }
    if let Some(pos) = window.iter().rposition(|c| *c == '\n') {
        return floor + pos + 1;
    }
    if let Some(pos) = window.iter().rposition(|c| c.is_whitespace()) {
        return floor + pos + 1;
    }
    end
}

/// Index an uploaded document: extract, chunk, persist chunks, embed each, and
/// flip status to ready/error. Runs detached from the upload request.
pub async fn index(state: &AppState, user_id: &str, doc_id: &str, filename: &str, mime: &str, bytes: Vec<u8>) {
    let result = index_inner(&state.db, &state.http_client, user_id, doc_id, filename, mime, &bytes).await;
    let (status, error) = match &result {
        Ok(chunks) => {
            state
                .log("documents", "info", format!("Indexed '{filename}' ({chunks} chunks)"))
                .await;
            ("ready", None)
        }
        Err(e) => {
            state
                .log("documents", "warn", format!("Indexing '{filename}' failed: {e}"))
                .await;
            ("error", Some(e.to_string()))
        }
    };
    if let Err(e) = db::documents::set_status(&state.db, doc_id, status, error.as_deref()).await {
        tracing::warn!("failed to set document status: {e}");
    }
}

async fn index_inner(
    pool: &SqlitePool,
    client: &reqwest::Client,
    user_id: &str,
    doc_id: &str,
    filename: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<usize> {
    let text = extract_text(filename, mime, bytes)?;
    let pieces = chunk(&text);
    let total = pieces.len();

    for (seq, piece) in pieces.into_iter().enumerate() {
        let chunk_id = db::documents::insert_chunk(pool, doc_id, user_id, seq as i64, &piece).await?;
        // Embedding failures leave the chunk searchable via the LIKE fallback;
        // the document still becomes ready.
        match embeddings::embed(pool, client, &piece).await {
            Ok(vec) => {
                db::documents::set_chunk_embedding(pool, &chunk_id, &embeddings::to_blob(&vec))
                    .await?;
            }
            Err(e) => tracing::warn!("chunk embedding failed (text search still works): {e}"),
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_respects_paragraphs_and_overlap() {
        let para = "Lorem ipsum dolor sit amet. ".repeat(30); // ~840 chars
        let text = format!("{para}\n\n{para}\n\n{para}");
        let chunks = chunk(&text);
        assert!(chunks.len() >= 2, "expected multiple chunks, got {}", chunks.len());
        // Every chunk within size bounds.
        for c in &chunks {
            assert!(c.chars().count() <= CHUNK_SIZE);
        }
        // All the text is represented (overlap means total >= original).
        let total: usize = chunks.iter().map(|c| c.chars().count()).sum();
        assert!(total >= text.trim().chars().count() - chunks.len() * 2);
    }

    #[test]
    fn chunking_short_text_is_one_chunk() {
        assert_eq!(chunk("hello world"), vec!["hello world"]);
        assert!(chunk("   ").is_empty());
    }

    #[test]
    fn extract_rejects_unknown_types() {
        assert!(extract_text("a.bin", "application/octet-stream", b"\x00\x01").is_err());
        assert!(extract_text("a.txt", "text/plain", b"").is_err()); // empty
    }

    #[test]
    fn extract_handles_text_and_html() {
        assert_eq!(extract_text("a.md", "text/markdown", b"# Title").unwrap(), "# Title");
        assert_eq!(
            extract_text("a.html", "text/html", b"<p>Hello</p><script>x</script>").unwrap(),
            "Hello"
        );
    }
}
