//! Helpdesk HTTP endpoints used by the UI directly (not the chat agent's
//! tools). Currently just a client/contact lookup so approval cards can render
//! human-readable client + requester names with a picker.

use axum::{
    extract::{Query, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::routes::auth::CurrentUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ClientsQuery {
    /// Which named helpdesk instance to query (omit = default/sole).
    #[serde(default)]
    integration: Option<String>,
    /// Optional server-side name filter.
    #[serde(default)]
    search: Option<String>,
}

// GET /api/helpdesk/clients — list clients, each with their contacts (users),
// for the create-ticket approval card's client/requester dropdowns. Trimmed to
// id/name(/email) so the payload stays small.
pub async fn list_clients(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<ClientsQuery>,
) -> AppResult<Json<Value>> {
    let user_id = user.id.as_str();
    // Pull a generous page so the dropdown holds the whole client list rather
    // than the API's default first page.
    let mut path = "/clients?per_page=500".to_string();
    if let Some(s) = q.search.as_deref().filter(|s| !s.is_empty()) {
        path.push_str(&format!("&search={}", urlencoding::encode(s)));
    }
    let res = crate::integrations::helpdesk::request(
        &state,
        user_id,
        q.integration.as_deref(),
        reqwest::Method::GET,
        &path,
        None,
    )
    .await
    .map_err(AppError::Internal)?;

    let clients: Vec<Value> = res["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let id = c["id"].as_i64()?;
                    let name = c["name"].as_str().unwrap_or("");
                    let users: Vec<Value> = c["users"]
                        .as_array()
                        .map(|us| {
                            us.iter()
                                .filter_map(|u| {
                                    let uid = u["id"].as_i64()?;
                                    Some(json!({
                                        "id": uid,
                                        "name": u["name"].as_str().unwrap_or(""),
                                        "email": u["email"].as_str().unwrap_or(""),
                                    }))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(json!({ "id": id, "name": name, "users": users }))
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(json!({ "clients": clients })))
}
