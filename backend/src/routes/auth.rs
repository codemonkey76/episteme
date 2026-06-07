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
const RECOVERY_CODES: usize = 8;

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

// ── TOTP helpers ────────────────────────────────────────────────────────────

fn build_totp(secret_b32: &str, account: &str) -> AppResult<totp_rs::TOTP> {
    let bytes = totp_rs::Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("bad TOTP secret: {e:?}")))?;
    totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1, // what authenticator apps actually implement
        6,
        1, // ±1 step of clock skew
        30,
        bytes,
        Some("Episteme".to_string()),
        account.to_string(),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("TOTP init failed: {e}")))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// "xxxxx-xxxxx" lowercase hex — 40 bits, plenty against online guessing.
fn gen_recovery_code() -> AppResult<String> {
    let mut bytes = [0u8; 5];
    getrandom::fill(&mut bytes)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("rng failure: {e}")))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{}-{}", &hex[..5], &hex[5..]))
}

/// Check a second factor: a 6-digit TOTP code (timestep claimed atomically so
/// a still-valid code can't be replayed) or a single-use recovery code.
async fn verify_second_factor(
    state: &AppState,
    user: &db::auth::User,
    code: &str,
) -> AppResult<bool> {
    let Some(secret) = user.totp_secret.as_deref() else {
        return Ok(false);
    };
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() == 6 && compact.chars().all(|c| c.is_ascii_digit()) {
        let now = unix_now();
        let ok = build_totp(secret, &user.username)?
            .check(&compact, now);
        if !ok {
            return Ok(false);
        }
        return Ok(db::auth::claim_totp_step(&state.db, &user.id, (now / 30) as i64).await?);
    }
    // Recovery code: hyphen/case-insensitive.
    let norm: String =
        compact.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase();
    if norm.is_empty() {
        return Ok(false);
    }
    Ok(db::auth::use_recovery_code(&state.db, &user.id, &sha256_hex(&norm)).await?)
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

/// Short-lived session acting as `user_id`, with the admin recorded so the
/// UI can show a banner and offer the way back.
pub(crate) async fn start_impersonated_session(
    state: &AppState,
    user_id: &str,
    admin_id: &str,
) -> AppResult<Cookie<'static>> {
    let token = gen_token();
    let now = Utc::now();
    let expires = now + Duration::hours(1);
    db::auth::create_session_as(
        &state.db,
        &token,
        user_id,
        &now.to_rfc3339(),
        &expires.to_rfc3339(),
        Some(admin_id),
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

/// Layered after `require_auth` on admin-only routes.
pub async fn require_admin(req: Request, next: Next) -> Response {
    match req.extensions().get::<CurrentUser>() {
        Some(CurrentUser(user)) if user.is_admin() => next.run(req).await,
        // 403, not 401: the session is valid, the user just isn't an admin.
        // (A 401 here would make the frontend treat an impersonated member
        // session as logged-out the moment any admin poll fires.)
        _ => AppError::Forbidden("admin access required".into()).into_response(),
    }
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
    let mut role = None;
    let mut impersonator = None;
    if let Some(token) = jar.get(COOKIE_NAME).map(|c| c.value().to_string()) {
        let now = Utc::now().to_rfc3339();
        if let Ok(Some(user)) = db::auth::session_user(&state.db, &token, &now).await {
            authenticated = true;
            username = Some(user.username);
            role = Some(user.role);
            if let Ok(Some(admin)) = db::auth::session_impersonator(&state.db, &token).await {
                impersonator = Some(admin.username);
            }
        }
    }

    Ok(Json(json!({
        "setup_required": setup_required,
        "authenticated": authenticated,
        "username": username,
        "role": role,
        "impersonator": impersonator,
    })))
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    /// Second factor — a TOTP or recovery code. Only consulted at login, and
    /// only when the account has 2FA enabled.
    #[serde(default)]
    code: Option<String>,
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
    db::auth::create_user(&state.db, &id, username, &hash, &Utc::now().to_rfc3339(), "admin")
        .await?;

    let cookie = start_session(&state, &id).await?;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": username }))))
}

/// Public: is this invite code redeemable? Lets the register page show the
/// form (with the label as a greeting) or a clear invalid/expired message.
pub async fn check_invite(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(code): axum::extract::Path<String>,
) -> AppResult<Json<Value>> {
    let invite = db::invites::get_valid(&state.db, &code).await?;
    Ok(Json(json!({
        "valid": invite.is_some(),
        "label": invite.map(|i| i.label),
    })))
}

#[derive(Deserialize)]
pub struct RegisterBody {
    code: String,
    username: String,
    password: String,
}

/// Public: redeem a single-use invite — the invite is the approval, so the
/// account is active immediately and a session starts right away.
pub async fn register(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<RegisterBody>,
) -> AppResult<(CookieJar, Json<Value>)> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }
    if body.password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if db::auth::get_user_by_username(&state.db, username).await?.is_some() {
        return Err(AppError::Conflict("that username is taken".into()));
    }
    // Validate before creating; claim atomically after, so a racing second
    // registration on the same code can't slip through.
    if db::invites::get_valid(&state.db, &body.code).await?.is_none() {
        return Err(AppError::Unauthorized("invite code is invalid or expired".into()));
    }

    let id = Uuid::new_v4().to_string();
    let hash = hash_password(&body.password)?;
    db::auth::create_user(&state.db, &id, username, &hash, &Utc::now().to_rfc3339(), "member")
        .await?;
    if !db::invites::mark_used(&state.db, &body.code, &id).await? {
        // Lost the race — roll the account back.
        let _ = db::auth::delete_user(&state.db, &id).await;
        return Err(AppError::Unauthorized("invite code is invalid or expired".into()));
    }
    state
        .log("auth", "info", format!("invite redeemed: {username} joined"))
        .await;

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
    if user.status != "active" {
        return Err(AppError::Unauthorized("this account is disabled".into()));
    }

    // Second factor. The password alone never starts a session on a 2FA
    // account: without a code the client is told to ask for one (200, not
    // 401 — the credentials were right); with a wrong code it's a plain 401.
    if user.totp_secret.is_some() {
        let code = body.code.as_deref().map(str::trim).unwrap_or("");
        if code.is_empty() {
            return Ok((jar, Json(json!({ "ok": false, "totp_required": true }))));
        }
        if !verify_second_factor(&state, &user, code).await? {
            state
                .log("auth", "warn", format!("2FA failed for {}", user.username))
                .await;
            return Err(AppError::Unauthorized("invalid two-factor code".into()));
        }
    }

    let cookie = start_session(&state, &user.id).await?;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": user.username }))))
}

// ── Two-factor enrollment (all behind require_auth) ─────────────────────────

/// Protected: start TOTP enrollment. The secret stays pending — and the
/// account stays 2FA-off — until the first code verifies via totp_enable.
pub async fn totp_setup(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    if user.totp_secret.is_some() {
        return Err(AppError::Conflict("two-factor authentication is already enabled".into()));
    }
    let secret = totp_rs::Secret::generate_secret();
    let b32 = secret.to_encoded().to_string();
    db::auth::set_totp_pending(&state.db, &user.id, &b32).await?;

    let url = build_totp(&b32, &user.username)?.get_url();
    // QR as inline SVG: nothing to host, nothing rasterized, themable by CSS.
    let qr_svg = qrcode::QrCode::new(url.as_bytes())
        .map(|qr| {
            qr.render::<qrcode::render::svg::Color>()
                .min_dimensions(180, 180)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default();
    Ok(Json(json!({ "secret": b32, "otpauth_url": url, "qr_svg": qr_svg })))
}

#[derive(Deserialize)]
pub struct TotpEnable {
    code: String,
}

/// Protected: verify the first code against the pending secret, switch 2FA
/// on, and hand back the recovery codes — shown exactly once.
pub async fn totp_enable(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<TotpEnable>,
) -> AppResult<Json<Value>> {
    let pending = user
        .totp_pending
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("no enrollment in progress — start setup first".into()))?;
    let now = unix_now();
    if !build_totp(pending, &user.username)?.check(body.code.trim(), now) {
        return Err(AppError::Unauthorized(
            "that code didn't match — try the next one from your app".into(),
        ));
    }
    if !db::auth::enable_totp(&state.db, &user.id).await? {
        return Err(AppError::BadRequest("no enrollment in progress — start setup first".into()));
    }
    // The enrollment code is spent too: claim its timestep.
    let _ = db::auth::claim_totp_step(&state.db, &user.id, (now / 30) as i64).await;

    let mut codes = Vec::with_capacity(RECOVERY_CODES);
    for _ in 0..RECOVERY_CODES {
        codes.push(gen_recovery_code()?);
    }
    let hashes: Vec<String> =
        codes.iter().map(|c| sha256_hex(&c.replace('-', ""))).collect();
    db::auth::replace_recovery_codes(&state.db, &user.id, &hashes).await?;

    state
        .log("auth", "info", format!("2FA enabled for {}", user.username))
        .await;
    Ok(Json(json!({ "ok": true, "recovery_codes": codes })))
}

#[derive(Deserialize)]
pub struct TotpDisable {
    password: String,
}

/// Protected: switch 2FA off. Requires the password, not a TOTP code — this
/// is also the recovery path for a lost authenticator (used with a recovery-
/// code login).
pub async fn totp_disable(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<TotpDisable>,
) -> AppResult<Json<Value>> {
    if !verify_password(&user.password_hash, &body.password) {
        return Err(AppError::Unauthorized("password is incorrect".into()));
    }
    db::auth::disable_totp(&state.db, &user.id).await?;
    state
        .log("auth", "info", format!("2FA disabled for {}", user.username))
        .await;
    Ok(Json(json!({ "ok": true })))
}

/// Protected: 2FA state for the Settings panel.
pub async fn totp_status(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> AppResult<Json<Value>> {
    let recovery_left = if user.totp_secret.is_some() {
        db::auth::recovery_codes_left(&state.db, &user.id).await?
    } else {
        0
    };
    Ok(Json(json!({
        "enabled": user.totp_secret.is_some(),
        "recovery_codes_left": recovery_left,
    })))
}

/// Protected: end an impersonated session and return to the admin account.
pub async fn stop_impersonating(
    State(state): State<Arc<AppState>>,
    jar: CookieJar,
) -> AppResult<(CookieJar, Json<Value>)> {
    let token = jar
        .get(COOKIE_NAME)
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::Unauthorized("not signed in".into()))?;
    let admin = db::auth::session_impersonator(&state.db, &token)
        .await?
        .ok_or_else(|| AppError::BadRequest("not impersonating".into()))?;

    db::auth::delete_session(&state.db, &token).await?;
    let cookie = start_session(&state, &admin.id).await?;
    state
        .log("auth", "info", format!("impersonation ended; back to {}", admin.username))
        .await;
    Ok((jar.add(cookie), Json(json!({ "ok": true, "username": admin.username }))))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_round_trip_with_skew() {
        let secret = totp_rs::Secret::generate_secret();
        let b32 = secret.to_encoded().to_string();
        let totp = build_totp(&b32, "shane").unwrap();

        let now = unix_now();
        let code = totp.generate(now);
        assert!(totp.check(&code, now));
        // ±1 step of clock skew is tolerated; ±2 is not.
        assert!(totp.check(&code, now + 30));
        assert!(!totp.check(&code, now + 90));
        assert!(!totp.check("000000", now) || code == "000000");

        // The otpauth URL carries issuer + account for the authenticator app.
        let url = totp.get_url();
        assert!(url.starts_with("otpauth://totp/"));
        assert!(url.contains("Episteme") && url.contains("shane"));
    }

    #[test]
    fn recovery_codes_are_well_formed_and_hash_stably() {
        let code = gen_recovery_code().unwrap();
        assert_eq!(code.len(), 11);
        assert_eq!(code.chars().filter(|c| *c == '-').count(), 1);
        assert!(code.replace('-', "").chars().all(|c| c.is_ascii_hexdigit()));

        // Login normalizes case/hyphens before hashing — these must agree.
        let norm: String = code
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        assert_eq!(sha256_hex(&norm), sha256_hex(&code.replace('-', "")));
        assert_eq!(sha256_hex("abc").len(), 64);
    }
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
