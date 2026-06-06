use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

/// Decoded upload cap — generous for text/PDF sources.
const UPLOAD_MAX: usize = 25 * 1024 * 1024;

// GET /api/documents
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let documents = db::documents::list(&state.db, &user.id).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "documents": documents })))
}

/// Upload body mirrors the email AttachmentUpload shape: raw bytes base64-
/// encoded in JSON (no `data:` prefix), so no multipart machinery is needed.
#[derive(Deserialize)]
pub struct UploadDocument {
    filename: String,
    content_type: String,
    content_bytes: String,
}

// POST /api/documents — store the row, then index (extract/chunk/embed) detached.
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<UploadDocument>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let filename = body.filename.trim();
    if filename.is_empty() {
        return Err(AppError::BadRequest("filename is required".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.content_bytes.trim())
        .map_err(|e| AppError::BadRequest(format!("upload is not valid base64: {e}")))?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("file is empty".into()));
    }
    if bytes.len() > UPLOAD_MAX {
        return Err(AppError::BadRequest("file exceeds the 25 MB upload limit".into()));
    }

    let doc = db::documents::insert(&state.db, &user.id, filename, &body.content_type, bytes.len() as i64)
        .await
        .map_err(AppError::Internal)?;

    let st = Arc::clone(&state);
    let user_id = user.id.clone();
    let doc_id = doc.id.clone();
    let fname = filename.to_string();
    let mime = body.content_type.clone();
    tokio::spawn(async move {
        crate::documents::index(&st, &user_id, &doc_id, &fname, &mime, bytes).await;
    });

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "document": doc }))))
}

// DELETE /api/documents/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::documents::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
