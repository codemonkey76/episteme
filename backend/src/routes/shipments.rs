use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;

use axum::Extension;

use crate::db::{self, shipments::ShipmentPatch};
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

/// Decoded photo cap. Phone cameras produce 3–8 MB shots; 12 MB leaves room
/// without inviting someone to park a video in the database.
const PHOTO_MAX: usize = 12 * 1024 * 1024;

#[derive(Deserialize)]
pub struct ListQuery {
    /// A specific status, "active" (the default view — not yet delivered or
    /// cancelled), or "all".
    status: Option<String>,
    q: Option<String>,
    limit: Option<i64>,
}

// GET /api/shipments?status=&q=&limit=
pub async fn list(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let status = params
        .status
        .as_deref()
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
    let q = params.q.as_deref().filter(|s| !s.is_empty());
    let shipments = db::shipments::list(&state.db, &user.id, status, q, params.limit.unwrap_or(200))
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "shipments": shipments })))
}

/// Tracking numbers are unique per user, so reusing one is a name clash rather
/// than a server fault — say so instead of leaking "database error: UNIQUE
/// constraint failed".
fn save_error(e: anyhow::Error) -> AppError {
    if e.to_string().contains("UNIQUE constraint failed: shipments.user_id, shipments.tracking_number")
    {
        return AppError::Conflict("another shipment already has that tracking number".into());
    }
    AppError::Internal(e)
}

/// Reject a status the CHECK constraint would refuse, with a message naming
/// what's allowed rather than surfacing a raw SQLite error.
fn status_or_err(value: &str) -> Result<&'static str, AppError> {
    db::shipments::normalize_status(value).ok_or_else(|| {
        AppError::BadRequest(format!(
            "invalid status '{value}' — expected one of: ordered, in_transit, \
             out_for_delivery, delivered, exception, cancelled"
        ))
    })
}

#[derive(Deserialize)]
pub struct CreateShipment {
    label: String,
    description: Option<String>,
    carrier: Option<String>,
    tracking_number: Option<String>,
    tracking_url: Option<String>,
    merchant: Option<String>,
    order_ref: Option<String>,
    status: Option<String>,
    eta: Option<String>,
}

/// Trim, and treat an empty string as absent — the forms post "" for untouched
/// optional fields, and a blank tracking number would collide with the partial
/// unique index on the second one saved.
fn clean(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

// POST /api/shipments
pub async fn create(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<CreateShipment>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let label = body.label.trim();
    if label.is_empty() {
        return Err(AppError::BadRequest("label is required".into()));
    }
    let status = match clean(&body.status) {
        Some(s) => status_or_err(s)?,
        None => "ordered",
    };
    let shipment = db::shipments::insert(
        &state.db,
        &user.id,
        label,
        clean(&body.description),
        clean(&body.carrier),
        clean(&body.tracking_number),
        clean(&body.tracking_url),
        clean(&body.merchant),
        clean(&body.order_ref),
        status,
        clean(&body.eta),
        "manual",
        None,
    )
    .await
    .map_err(save_error)?;
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "shipment": shipment }))))
}

/// Distinguishes a JSON field that's absent (outer None — leave unchanged)
/// from one explicitly set to null (Some(None) — clear it).
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// Partial update — absent fields unchanged; the optional ones accept null to
/// clear.
#[derive(Deserialize)]
pub struct UpdateShipment {
    label: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    carrier: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    tracking_number: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    tracking_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    merchant: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    order_ref: Option<Option<String>>,
    status: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    eta: Option<Option<String>>,
}

/// Normalize a clearable text field: null or blank clears it.
fn clean_patch(v: Option<Option<String>>) -> Option<Option<String>> {
    v.map(|inner| inner.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

// PUT /api/shipments/:id
pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UpdateShipment>,
) -> AppResult<Json<serde_json::Value>> {
    let status = match body.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(status_or_err(s)?.to_string()),
        None => None,
    };
    let patch = ShipmentPatch {
        label: body.label.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        description: clean_patch(body.description),
        carrier: clean_patch(body.carrier),
        tracking_number: clean_patch(body.tracking_number),
        tracking_url: clean_patch(body.tracking_url),
        merchant: clean_patch(body.merchant),
        order_ref: clean_patch(body.order_ref),
        status,
        eta: clean_patch(body.eta),
    };
    let shipment = db::shipments::update(&state.db, &user.id, &id, patch)
        .await
        .map_err(save_error)?
        .ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({ "shipment": shipment })))
}

// DELETE /api/shipments/:id
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    db::shipments::delete(&state.db, &user.id, &id).await.map_err(AppError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateEvent {
    detail: String,
    /// Optional status this update moves the shipment to.
    status: Option<String>,
    /// RFC3339; defaults to now.
    occurred_at: Option<String>,
}

// POST /api/shipments/:id/events — a manual note or status update on the timeline.
pub async fn add_event(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<CreateEvent>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let detail = body.detail.trim();
    if detail.is_empty() {
        return Err(AppError::BadRequest("detail is required".into()));
    }
    // Confirms the shipment is this user's before writing a child row.
    db::shipments::get(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    let status = match body.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(status_or_err(s)?),
        None => None,
    };
    let occurred_at = body
        .occurred_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    let event = db::shipments::add_event(&state.db, &id, status, detail, &occurred_at, "manual")
        .await
        .map_err(AppError::Internal)?;
    // A manual status note moves the shipment too — the timeline and the card
    // shouldn't disagree.
    if let Some(s) = status {
        db::shipments::update(
            &state.db,
            &user.id,
            &id,
            ShipmentPatch { status: Some(s.to_string()), ..Default::default() },
        )
        .await
        .map_err(AppError::Internal)?;
    }
    Ok((StatusCode::CREATED, Json(serde_json::json!({ "event": event }))))
}

/// Photo upload mirrors the document/attachment shape: raw bytes base64-encoded
/// in JSON (no `data:` prefix), so no multipart machinery is needed.
#[derive(Deserialize)]
pub struct UploadPhoto {
    content_type: String,
    content_bytes: String,
}

// PUT /api/shipments/:id/photo
pub async fn set_photo(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
    Json(body): Json<UploadPhoto>,
) -> AppResult<StatusCode> {
    let mime = body.content_type.trim();
    if !mime.starts_with("image/") {
        return Err(AppError::BadRequest("photo must be an image".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.content_bytes.trim())
        .map_err(|e| AppError::BadRequest(format!("photo is not valid base64: {e}")))?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest("photo is empty".into()));
    }
    if bytes.len() > PHOTO_MAX {
        return Err(AppError::BadRequest("photo exceeds the 12 MB limit".into()));
    }
    let ok = db::shipments::set_photo(&state.db, &user.id, &id, &bytes, mime)
        .await
        .map_err(AppError::Internal)?;
    if !ok {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/shipments/:id/photo
pub async fn delete_photo(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let ok = db::shipments::clear_photo(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?;
    if !ok {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// GET /api/shipments/:id/photo — raw bytes for an <img> tag.
pub async fn get_photo(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let (bytes, mime) = db::shipments::photo(&state.db, &user.id, &id)
        .await
        .map_err(AppError::Internal)?
        .ok_or(AppError::NotFound)?;

    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, "inline")
        // The photo can be replaced in place, so revalidate rather than cache
        // hard — a swapped picture must not keep showing the old one.
        .header(header::CACHE_CONTROL, "private, no-cache")
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(e.into()))
}
