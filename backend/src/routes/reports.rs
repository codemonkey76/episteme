use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use std::sync::Arc;

use axum::Extension;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

// GET /api/reports — the user's reports, newest first (metadata only).
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<serde_json::Value>> {
    let reports =
        db::reports::list_for_user(&state.db, &user.id, 100).await.map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "reports": reports })))
}

// GET /api/reports/:id/html — the self-contained report document, served raw
// for iframes / open-in-new-tab (cookie-authed like every /api route).
pub async fn html(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let html = db::reports::get_html(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(html))
        .map_err(|e| AppError::Internal(e.into()))
}

// DELETE /api/reports/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::reports::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// 256 bits of URL-safe token — unguessable, so the link itself is the
/// capability (same model as the session cookie).
fn gen_share_token() -> String {
    use uuid::Uuid;
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

// POST /api/reports/:id/share — mint (or return the existing) public link.
// The recipient needs no account. Idempotent: re-sharing keeps the same token,
// so a link already handed out stays valid.
pub async fn share(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let existing =
        db::reports::share_token(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    // Outer None → not their report (or doesn't exist).
    let current = existing.ok_or(AppError::NotFound)?;
    let token = match current {
        Some(t) => t,
        None => {
            let t = gen_share_token();
            db::reports::set_share_token(&state.db, &user.id, &id, Some(&t))
                .await
                .map_err(AppError::Internal)?;
            state.log("reports", "info", format!("report shared: {id}")).await;
            t
        }
    };
    // A relative path — the frontend prepends its own origin (robust behind a
    // reverse proxy, which the server can't reliably infer).
    Ok(Json(serde_json::json!({ "token": token, "path": format!("/shared/{token}") })))
}

// DELETE /api/reports/:id/share — revoke the link (existing recipients lose
// access immediately).
pub async fn unshare(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let ok = db::reports::set_share_token(&state.db, &user.id, &id, None)
        .await
        .map_err(AppError::Internal)?;
    if !ok {
        return Err(AppError::NotFound);
    }
    state.log("reports", "info", format!("report share revoked: {id}")).await;
    Ok(StatusCode::NO_CONTENT)
}

// GET /shared/:token — PUBLIC: the self-contained report, no account required.
// Reports are static HTML (inline CSS/SVG, data-URI images, no scripts); a
// strict CSP and nosniff are belt-and-braces against the model-generated body.
pub async fn shared(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> AppResult<Response> {
    let (_title, html) =
        db::reports::get_shared(&state.db, &token).await.map_err(AppError::Internal)?.ok_or(
            // Wrong/revoked token: a plain 404, never distinguishing "never
            // existed" from "revoked".
            AppError::NotFound,
        )?;
    Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:",
        )
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::from(html))
        .map_err(|e| AppError::Internal(e.into()))
}

#[derive(serde::Deserialize)]
pub struct StartResearch {
    topic: String,
    #[serde(default)]
    depth: Option<String>,
    #[serde(default)]
    provider: Option<String>,
}

// POST /api/research — the Reports window's "New research" launcher.
pub async fn start_research(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<StartResearch>,
) -> AppResult<Json<serde_json::Value>> {
    let (job_id, session_id) = crate::research::launch(
        &state,
        &user.id,
        &body.topic,
        body.depth.as_deref().unwrap_or("standard"),
        body.provider.as_deref().unwrap_or(""),
    )
    .await
    // Launch failures are user-facing config problems (empty topic, unknown
    // provider) — surface them as 400s with the message.
    .map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(serde_json::json!({ "job_id": job_id, "session_id": session_id })))
}
