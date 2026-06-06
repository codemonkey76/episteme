use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Document {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
    /// Populated by `list` via subquery; 0 while indexing.
    #[sqlx(default)]
    pub chunk_count: i64,
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    filename: &str,
    mime: &str,
    size: i64,
) -> Result<Document> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO documents (id, user_id, filename, mime, size, status, created_at)
         VALUES (?, ?, ?, ?, ?, 'indexing', ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(filename)
    .bind(mime)
    .bind(size)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Document {
        id,
        filename: filename.to_string(),
        mime: mime.to_string(),
        size,
        status: "indexing".to_string(),
        error_message: None,
        created_at: now,
        chunk_count: 0,
    })
}

pub async fn list(pool: &SqlitePool, user_id: &str) -> Result<Vec<Document>> {
    Ok(sqlx::query_as::<_, Document>(
        "SELECT d.id, d.filename, d.mime, d.size, d.status, d.error_message, d.created_at,
                (SELECT COUNT(*) FROM document_chunks c WHERE c.document_id = d.id) AS chunk_count
         FROM documents d WHERE d.user_id = ? ORDER BY d.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

pub async fn set_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE documents SET status = ?, error_message = ? WHERE id = ?")
        .bind(status)
        .bind(error_message)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a document; chunks go with it (ON DELETE CASCADE).
pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM documents WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_chunk(
    pool: &SqlitePool,
    document_id: &str,
    user_id: &str,
    seq: i64,
    content: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO document_chunks (id, document_id, user_id, seq, content)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(document_id)
    .bind(user_id)
    .bind(seq)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn set_chunk_embedding(pool: &SqlitePool, id: &str, blob: &[u8]) -> Result<()> {
    sqlx::query("UPDATE document_chunks SET embedding = ? WHERE id = ?")
        .bind(blob)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// All of a user's chunks (with embeddings) joined to their document name —
/// the candidate pool for search_documents. Capped defensively.
#[derive(Debug, sqlx::FromRow)]
pub struct SearchChunk {
    pub filename: String,
    pub seq: i64,
    pub content: String,
    pub embedding: Option<Vec<u8>>,
}

pub async fn search_pool(pool: &SqlitePool, user_id: &str, limit: i64) -> Result<Vec<SearchChunk>> {
    Ok(sqlx::query_as::<_, SearchChunk>(
        "SELECT d.filename, c.seq, c.content, c.embedding
         FROM document_chunks c JOIN documents d ON d.id = c.document_id
         WHERE c.user_id = ? AND d.status = 'ready'
         LIMIT ?",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}
