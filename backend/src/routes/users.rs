//! Admin-only user and invite management. All routes here sit behind
//! `require_auth` + `require_admin`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

// GET /api/users
pub async fn list(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let users: Vec<Value> = db::auth::list_users(&state.db)
        .await?
        .into_iter()
        .map(|u| {
            json!({
                "id": u.id,
                "username": u.username,
                "role": u.role,
                "status": u.status,
                "created_at": u.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "users": users })))
}

async fn target_member(
    state: &AppState,
    actor: &db::auth::User,
    id: &str,
) -> AppResult<db::auth::User> {
    let target = db::auth::get_user(&state.db, id).await?.ok_or(AppError::NotFound)?;
    if target.id == actor.id {
        return Err(AppError::BadRequest("you can't do that to your own account".into()));
    }
    if target.is_admin() {
        return Err(AppError::BadRequest("the admin account can't be modified".into()));
    }
    Ok(target)
}

// POST /api/users/:id/disable
pub async fn disable(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target_member(&state, &actor, &id).await?;
    db::auth::set_status(&state.db, &target.id, "disabled").await?;
    db::auth::delete_user_sessions(&state.db, &target.id).await?;
    state.log("auth", "warn", format!("user disabled: {}", target.username)).await;
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/users/:id/enable
pub async fn enable(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target_member(&state, &actor, &id).await?;
    db::auth::set_status(&state.db, &target.id, "active").await?;
    state.log("auth", "info", format!("user enabled: {}", target.username)).await;
    Ok(StatusCode::NO_CONTENT)
}

// DELETE /api/users/:id — removes the account AND all their data.
pub async fn delete(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let target = target_member(&state, &actor, &id).await?;
    db::auth::delete_user(&state.db, &target.id).await?;
    state.log("auth", "warn", format!("user deleted: {}", target.username)).await;
    Ok(StatusCode::NO_CONTENT)
}

// POST /api/users/:id/impersonate — act as a member to set things up for
// them (e.g. connect their mailbox). Session lasts 1 hour; the banner and
// stop endpoint come from the impersonator recorded on the session.
pub async fn impersonate(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    jar: axum_extra::extract::CookieJar,
    Path(id): Path<String>,
) -> AppResult<(axum_extra::extract::CookieJar, Json<Value>)> {
    let target = target_member(&state, &actor, &id).await?;
    if target.status != "active" {
        return Err(AppError::BadRequest("enable the account first".into()));
    }
    let cookie =
        crate::routes::auth::start_impersonated_session(&state, &target.id, &actor.id).await?;
    state
        .log("auth", "warn", format!("{} is impersonating {}", actor.username, target.username))
        .await;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": target.username }))))
}

// ── Invites ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateInvite {
    #[serde(default)]
    label: String,
}

// POST /api/admin/invites
pub async fn create_invite(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateInvite>,
) -> AppResult<Json<Value>> {
    let invite = db::invites::create(&state.db, body.label.trim()).await?;
    state
        .log("auth", "info", format!("invite created: {}", if invite.label.is_empty() { &invite.code } else { &invite.label }))
        .await;
    Ok(Json(json!({ "invite": invite })))
}

// GET /api/admin/invites — pending + redeemed, with the redeemer's username.
pub async fn list_invites(State(state): State<Arc<AppState>>) -> AppResult<Json<Value>> {
    let invites = db::invites::list(&state.db).await?;
    let users = db::auth::list_users(&state.db).await?;
    let name_of = |id: &Option<String>| {
        id.as_deref()
            .and_then(|uid| users.iter().find(|u| u.id == uid))
            .map(|u| u.username.clone())
    };
    let rows: Vec<Value> = invites
        .iter()
        .map(|i| {
            json!({
                "code": i.code,
                "label": i.label,
                "created_at": i.created_at,
                "expires_at": i.expires_at,
                "used_by": name_of(&i.used_by),
                "used_at": i.used_at,
            })
        })
        .collect();
    Ok(Json(json!({ "invites": rows })))
}

// DELETE /api/admin/invites/:code — revoke an unused invite.
pub async fn revoke_invite(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> AppResult<StatusCode> {
    db::invites::delete(&state.db, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}
