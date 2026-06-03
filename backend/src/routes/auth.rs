use std::sync::Arc;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const COOKIE_NAME: &str = "episteme_session";
const SESSION_DAYS: i64 = 30;
const MIN_PASSWORD_LEN: usize = 8;

/// The authenticated user, injected into request extensions by `require_auth`.
#[derive(Clone)]
pub struct CurrentUser(pub db::auth::User);

// ── Password + token helpers ────────────────────────────────────────────────

fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("hash failed: {e}")))?
        .to_string();
    Ok(hash)
}

fn verify_password(hash: &str, password: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// 256 bits of opaque session entropy (two v4 UUIDs, hex).
fn gen_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Build the session cookie. `Secure` is on by default; set AUTH_COOKIE_INSECURE
/// (e.g. for plain-http local dev) to allow the cookie over http.
fn session_cookie(token: String) -> Cookie<'static> {
    let secure = std::env::var("AUTH_COOKIE_INSECURE").is_err();
    Cookie::build((COOKIE_NAME, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(time::Duration::days(SESSION_DAYS))
        .build()
}

fn cleared_cookie() -> Cookie<'static> {
    let secure = std::env::var("AUTH_COOKIE_INSECURE").is_err();
    Cookie::build((COOKIE_NAME, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .max_age(time::Duration::seconds(0))
        .build()
}

async fn start_session(state: &AppState, user_id: &str) -> AppResult<Cookie<'static>> {
    let token = gen_token();
    let now = Utc::now();
    let expires = now + Duration::days(SESSION_DAYS);
    db::auth::create_session(
        &state.db,
        &token,
        user_id,
        &now.to_rfc3339(),
        &expires.to_rfc3339(),
    )
    .await?;
    Ok(session_cookie(token))
}

// ── Middleware ──────────────────────────────────────────────────────────────

/// Reject any request without a valid session cookie. On success, the resolved
/// user is placed in request extensions as `CurrentUser`.
pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    if let Some(token) = jar.get(COOKIE_NAME).map(|c| c.value().to_string()) {
        let now = Utc::now().to_rfc3339();
        if let Ok(Some(user)) = db::auth::session_user(&state.db, &token, &now).await {
            req.extensions_mut().insert(CurrentUser(user));
            return next.run(req).await;
        }
    }
    AppError::Unauthorized("authentication required".into()).into_response()
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Public: tells the frontend whether to show setup, login, or the app.
pub async fn status(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> AppResult<Json<Value>> {
    let setup_required = db::auth::count_users(&state.db).await? == 0;

    let mut authenticated = false;
    let mut username = None;
    if let Some(token) = jar.get(COOKIE_NAME).map(|c| c.value().to_string()) {
        let now = Utc::now().to_rfc3339();
        if let Ok(Some(user)) = db::auth::session_user(&state.db, &token, &now).await {
            authenticated = true;
            username = Some(user.username);
        }
    }

    Ok(Json(json!({
        "setup_required": setup_required,
        "authenticated": authenticated,
        "username": username,
    })))
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

/// Public, but only valid while no account exists: creates the admin and logs in.
pub async fn setup(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> AppResult<(CookieJar, Json<Value>)> {
    if db::auth::count_users(&state.db).await? > 0 {
        return Err(AppError::Conflict("an account already exists".into()));
    }
    let username = body.username.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }
    if body.password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }

    let id = Uuid::new_v4().to_string();
    let hash = hash_password(&body.password)?;
    db::auth::create_user(&state.db, &id, username, &hash, &Utc::now().to_rfc3339()).await?;

    let cookie = start_session(&state, &id).await?;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": username }))))
}

/// Public: verify credentials and start a session.
pub async fn login(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> AppResult<(CookieJar, Json<Value>)> {
    let user = db::auth::get_user_by_username(&state.db, body.username.trim()).await?;
    // Always run a verification to keep timing roughly constant whether or not
    // the username exists.
    let ok = match &user {
        Some(u) => verify_password(&u.password_hash, &body.password),
        None => {
            verify_password(
                "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
                 b2d4Z3h4Z2h4Z2h4Z2h4Z2h4Z2h4Z2h4Z2h4Z2h4Z2g",
                &body.password,
            );
            false
        }
    };
    if !ok {
        return Err(AppError::Unauthorized("invalid username or password".into()));
    }
    let user = user.expect("ok implies user exists");

    let cookie = start_session(&state, &user.id).await?;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": user.username }))))
}

/// Public: clear the session (no-op if already logged out).
pub async fn logout(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> AppResult<(CookieJar, Json<Value>)> {
    if let Some(token) = jar.get(COOKIE_NAME).map(|c| c.value().to_string()) {
        db::auth::delete_session(&state.db, &token).await?;
    }
    Ok((jar.add(cleared_cookie()), Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
pub struct ChangePassword {
    current: String,
    next: String,
}

/// Protected: verify the current password and set a new one, invalidating all
/// existing sessions (the caller is logged out and must sign in again).
pub async fn change_password(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    jar: CookieJar,
    Json(body): Json<ChangePassword>,
) -> AppResult<(CookieJar, Json<Value>)> {
    if !verify_password(&user.password_hash, &body.current) {
        return Err(AppError::Unauthorized("current password is incorrect".into()));
    }
    if body.next.len() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    let hash = hash_password(&body.next)?;
    db::auth::update_password(&state.db, &user.id, &hash).await?;
    db::auth::delete_user_sessions(&state.db, &user.id).await?;
    Ok((jar.add(cleared_cookie()), Json(json!({ "ok": true }))))
}
