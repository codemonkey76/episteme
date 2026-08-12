//! Shipment tracking storage: what's on the way, where it's up to, and the
//! history of updates behind that. Rows come from three places — the user, the
//! chat agent, and the email categorizer's deliveries pass — so everything here
//! is written to tolerate partial information (a label and nothing else is a
//! valid shipment).

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Shipment {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub carrier: Option<String>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
    pub merchant: Option<String>,
    pub order_ref: Option<String>,
    pub status: String,
    pub eta: Option<String>,
    pub delivered_at: Option<String>,
    /// True when a photo is attached; the bytes are fetched separately by the
    /// photo route so list responses stay small.
    pub has_photo: bool,
    pub source: String,
    pub conversation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Filled in by `list`/`get`, not a column.
    #[sqlx(skip)]
    pub events: Vec<ShipmentEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ShipmentEvent {
    pub id: String,
    pub shipment_id: String,
    pub status: Option<String>,
    pub detail: String,
    pub occurred_at: String,
    pub source: String,
}

/// Fields that may be changed by an update; `None` leaves the column as-is,
/// `Some(None)` clears it.
#[derive(Debug, Default)]
pub struct ShipmentPatch {
    pub label: Option<String>,
    pub description: Option<Option<String>>,
    pub carrier: Option<Option<String>>,
    pub tracking_number: Option<Option<String>>,
    pub tracking_url: Option<Option<String>>,
    pub merchant: Option<Option<String>>,
    pub order_ref: Option<Option<String>>,
    pub status: Option<String>,
    pub eta: Option<Option<String>>,
}

/// What the categorizer's extraction pass pulled out of one shipping email.
#[derive(Debug, Default)]
pub struct Extracted {
    pub label: String,
    pub carrier: Option<String>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
    pub merchant: Option<String>,
    pub order_ref: Option<String>,
    pub status: Option<String>,
    pub eta: Option<String>,
    /// One-line description of this update, stored as the event detail.
    pub detail: String,
}

const COLS: &str = "id, label, description, carrier, tracking_number, tracking_url, merchant, \
                    order_ref, status, eta, delivered_at, photo_mime IS NOT NULL AS has_photo, \
                    source, conversation_id, created_at, updated_at";

/// Coerce a caller-supplied status to one the CHECK constraint accepts, so a
/// model inventing "shipped" can't fail the insert. Returns None if unknown.
pub fn normalize_status(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
        "ordered" | "confirmed" | "processing" | "placed" => Some("ordered"),
        "in_transit" | "shipped" | "dispatched" | "transit" | "on_the_way" => Some("in_transit"),
        "out_for_delivery" | "outfordelivery" | "with_courier" => Some("out_for_delivery"),
        "delivered" | "collected" | "picked_up" => Some("delivered"),
        "exception" | "delayed" | "failed" | "held" | "returned" => Some("exception"),
        "cancelled" | "canceled" => Some("cancelled"),
        _ => None,
    }
}

/// How far along a status is, for deciding whether an update moves a shipment
/// forward. Shipping mail arrives out of order often enough (a delayed
/// "dispatched" notice landing after "delivered") that a blind overwrite would
/// walk a delivered parcel backwards.
fn progress(status: &str) -> i32 {
    match status {
        "ordered" => 0,
        "in_transit" => 1,
        "out_for_delivery" => 2,
        "delivered" => 3,
        // Exceptions and cancellations are always news, whatever came before.
        _ => i32::MAX,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    label: &str,
    description: Option<&str>,
    carrier: Option<&str>,
    tracking_number: Option<&str>,
    tracking_url: Option<&str>,
    merchant: Option<&str>,
    order_ref: Option<&str>,
    status: &str,
    eta: Option<&str>,
    source: &str,
    conversation_id: Option<&str>,
) -> Result<Shipment> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let delivered_at = (status == "delivered").then(|| now.clone());
    sqlx::query(
        "INSERT INTO shipments (id, user_id, label, description, carrier, tracking_number,
                                tracking_url, merchant, order_ref, status, eta, delivered_at,
                                source, conversation_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(label)
    .bind(description)
    .bind(carrier)
    .bind(tracking_number)
    .bind(tracking_url)
    .bind(merchant)
    .bind(order_ref)
    .bind(status)
    .bind(eta)
    .bind(&delivered_at)
    .bind(source)
    .bind(conversation_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Shipment {
        id,
        label: label.to_string(),
        description: description.map(str::to_string),
        carrier: carrier.map(str::to_string),
        tracking_number: tracking_number.map(str::to_string),
        tracking_url: tracking_url.map(str::to_string),
        merchant: merchant.map(str::to_string),
        order_ref: order_ref.map(str::to_string),
        status: status.to_string(),
        eta: eta.map(str::to_string),
        delivered_at,
        has_photo: false,
        source: source.to_string(),
        conversation_id: conversation_id.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
        events: Vec::new(),
    })
}

/// List shipments, each with its event history. `status` filters to one status,
/// or pass "active" for everything not yet delivered or cancelled — the default
/// view, since a delivered parcel stops being interesting.
pub async fn list(
    pool: &SqlitePool,
    user_id: &str,
    status: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> Result<Vec<Shipment>> {
    let mut sql = format!("SELECT {COLS} FROM shipments WHERE user_id = ?");
    match status {
        Some("active") => sql.push_str(" AND status NOT IN ('delivered', 'cancelled')"),
        Some(_) => sql.push_str(" AND status = ?"),
        None => {}
    }
    if q.is_some() {
        sql.push_str(
            " AND (label LIKE ? OR description LIKE ? OR merchant LIKE ? OR tracking_number LIKE ?)",
        );
    }
    // Undelivered first, then soonest ETA (unknown last), then newest.
    sql.push_str(
        " ORDER BY CASE WHEN status IN ('delivered', 'cancelled') THEN 1 ELSE 0 END,
                  eta IS NULL, eta ASC, created_at DESC LIMIT ?",
    );

    let mut qb = sqlx::query_as::<_, Shipment>(&sql);
    qb = qb.bind(user_id);
    if let Some(s) = status.filter(|s| *s != "active") {
        qb = qb.bind(s.to_string());
    }
    if let Some(s) = q {
        let like = format!("%{s}%");
        qb = qb.bind(like.clone()).bind(like.clone()).bind(like.clone()).bind(like);
    }
    qb = qb.bind(limit);

    let mut shipments = qb.fetch_all(pool).await?;
    attach_events(pool, &mut shipments).await?;
    Ok(shipments)
}

pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<Shipment>> {
    let row = sqlx::query_as::<_, Shipment>(&format!(
        "SELECT {COLS} FROM shipments WHERE id = ? AND user_id = ?"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some(shipment) = row else { return Ok(None) };
    let mut one = vec![shipment];
    attach_events(pool, &mut one).await?;
    Ok(one.pop())
}

/// Load every event for the given shipments in one query and hang each on its
/// shipment (newest first).
async fn attach_events(pool: &SqlitePool, shipments: &mut [Shipment]) -> Result<()> {
    if shipments.is_empty() {
        return Ok(());
    }
    let placeholders = vec!["?"; shipments.len()].join(", ");
    let sql = format!(
        "SELECT id, shipment_id, status, detail, occurred_at, source FROM shipment_events
         WHERE shipment_id IN ({placeholders}) ORDER BY occurred_at DESC"
    );
    let mut qb = sqlx::query_as::<_, ShipmentEvent>(&sql);
    for s in shipments.iter() {
        qb = qb.bind(s.id.clone());
    }
    let events = qb.fetch_all(pool).await?;
    for e in events {
        if let Some(s) = shipments.iter_mut().find(|s| s.id == e.shipment_id) {
            s.events.push(e);
        }
    }
    Ok(())
}

/// Apply a partial update and return the resulting row (None if id unknown).
pub async fn update(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    patch: ShipmentPatch,
) -> Result<Option<Shipment>> {
    let Some(mut s) = get(pool, user_id, id).await? else {
        return Ok(None);
    };
    if let Some(v) = patch.label { s.label = v; }
    if let Some(v) = patch.description { s.description = v; }
    if let Some(v) = patch.carrier { s.carrier = v; }
    if let Some(v) = patch.tracking_number { s.tracking_number = v; }
    if let Some(v) = patch.tracking_url { s.tracking_url = v; }
    if let Some(v) = patch.merchant { s.merchant = v; }
    if let Some(v) = patch.order_ref { s.order_ref = v; }
    if let Some(v) = patch.eta { s.eta = v; }
    if let Some(v) = patch.status {
        // Stamp/clear the delivery time alongside the status so the two can't
        // disagree (e.g. reopening a parcel marked delivered by mistake).
        s.delivered_at = match (v.as_str(), s.delivered_at.take()) {
            ("delivered", Some(at)) => Some(at),
            ("delivered", None) => Some(Utc::now().to_rfc3339()),
            _ => None,
        };
        s.status = v;
    }
    s.updated_at = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE shipments SET label = ?, description = ?, carrier = ?, tracking_number = ?,
                              tracking_url = ?, merchant = ?, order_ref = ?, status = ?, eta = ?,
                              delivered_at = ?, updated_at = ?
         WHERE id = ? AND user_id = ?",
    )
    .bind(&s.label)
    .bind(&s.description)
    .bind(&s.carrier)
    .bind(&s.tracking_number)
    .bind(&s.tracking_url)
    .bind(&s.merchant)
    .bind(&s.order_ref)
    .bind(&s.status)
    .bind(&s.eta)
    .bind(&s.delivered_at)
    .bind(&s.updated_at)
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(Some(s))
}

/// Delete a shipment and its history. Events go first and explicitly: the
/// schema declares the foreign key, but SQLite only enforces `ON DELETE
/// CASCADE` with `PRAGMA foreign_keys = ON`, which this pool doesn't set.
pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM shipment_events
         WHERE shipment_id IN (SELECT id FROM shipments WHERE id = ? AND user_id = ?)",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM shipments WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Photo ──────────────────────────────────────────────────────────────────────

pub async fn set_photo(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    bytes: &[u8],
    mime: &str,
) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE shipments SET photo = ?, photo_mime = ?, updated_at = ? WHERE id = ? AND user_id = ?",
    )
    .bind(bytes)
    .bind(mime)
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn clear_photo(pool: &SqlitePool, user_id: &str, id: &str) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE shipments SET photo = NULL, photo_mime = NULL, updated_at = ?
         WHERE id = ? AND user_id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// The stored photo bytes and their mime type, if one is attached.
pub async fn photo(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<(Vec<u8>, String)>> {
    let row: Option<(Option<Vec<u8>>, Option<String>)> =
        sqlx::query_as("SELECT photo, photo_mime FROM shipments WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(bytes, mime)| Some((bytes?, mime?))))
}

// ── Events ─────────────────────────────────────────────────────────────────────

pub async fn add_event(
    pool: &SqlitePool,
    shipment_id: &str,
    status: Option<&str>,
    detail: &str,
    occurred_at: &str,
    source: &str,
) -> Result<ShipmentEvent> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO shipment_events (id, shipment_id, status, detail, occurred_at, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(shipment_id)
    .bind(status)
    .bind(detail)
    .bind(occurred_at)
    .bind(source)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(ShipmentEvent {
        id,
        shipment_id: shipment_id.to_string(),
        status: status.map(str::to_string),
        detail: detail.to_string(),
        occurred_at: occurred_at.to_string(),
        source: source.to_string(),
    })
}

// ── Email upsert ───────────────────────────────────────────────────────────────

/// Match a shipping email to an existing shipment, or create one. Matching is
/// by tracking number first (the same parcel keeps its number across carriers'
/// mail), then by the email thread it arrived in.
async fn find_match(
    pool: &SqlitePool,
    user_id: &str,
    tracking_number: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<Option<Shipment>> {
    if let Some(tn) = tracking_number.map(str::trim).filter(|s| !s.is_empty()) {
        let row = sqlx::query_as::<_, Shipment>(&format!(
            "SELECT {COLS} FROM shipments WHERE user_id = ? AND tracking_number = ?"
        ))
        .bind(user_id)
        .bind(tn)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Some(cid) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
        // Newest wins if a thread somehow spawned several.
        return Ok(sqlx::query_as::<_, Shipment>(&format!(
            "SELECT {COLS} FROM shipments WHERE user_id = ? AND conversation_id = ?
             ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(user_id)
        .bind(cid)
        .fetch_optional(pool)
        .await?);
    }
    Ok(None)
}

/// Outcome of folding a shipping email into the shipment list.
pub struct UpsertResult {
    pub shipment: Shipment,
    pub created: bool,
    /// False when the mail told us nothing new (same status, no new fields) —
    /// the caller uses this to stay quiet instead of notifying.
    pub changed: bool,
}

/// Fold one extracted shipping email into the shipment list: create the
/// shipment if it's new, otherwise fill in blanks and advance its status.
/// Details already known are never overwritten with worse ones — a later
/// "your parcel has shipped" mail that omits the ETA must not clear it.
pub async fn upsert_from_email(
    pool: &SqlitePool,
    user_id: &str,
    conversation_id: Option<&str>,
    ex: &Extracted,
    occurred_at: &str,
) -> Result<UpsertResult> {
    let tracking = ex.tracking_number.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let status = ex.status.as_deref().and_then(normalize_status).unwrap_or("in_transit");

    let Some(existing) = find_match(pool, user_id, tracking, conversation_id).await? else {
        let label = if ex.label.trim().is_empty() { "Incoming delivery" } else { ex.label.trim() };
        let shipment = insert(
            pool,
            user_id,
            label,
            None,
            ex.carrier.as_deref(),
            tracking,
            ex.tracking_url.as_deref(),
            ex.merchant.as_deref(),
            ex.order_ref.as_deref(),
            status,
            ex.eta.as_deref(),
            "email",
            conversation_id,
        )
        .await?;
        let event = add_event(pool, &shipment.id, Some(status), &ex.detail, occurred_at, "email").await?;
        return Ok(UpsertResult {
            shipment: Shipment { events: vec![event], ..shipment },
            created: true,
            changed: true,
        });
    };

    // Fill blanks only; a field we already hold is better evidence than a
    // re-extraction from a differently-worded email.
    fn fill(current: &Option<String>, incoming: Option<&str>) -> Option<Option<String>> {
        match (current, incoming.map(str::trim).filter(|s| !s.is_empty())) {
            (None, Some(v)) => Some(Some(v.to_string())),
            _ => None,
        }
    }
    let advanced = progress(status) > progress(&existing.status);
    let mut patch = ShipmentPatch {
        carrier: fill(&existing.carrier, ex.carrier.as_deref()),
        tracking_number: fill(&existing.tracking_number, tracking),
        tracking_url: fill(&existing.tracking_url, ex.tracking_url.as_deref()),
        merchant: fill(&existing.merchant, ex.merchant.as_deref()),
        order_ref: fill(&existing.order_ref, ex.order_ref.as_deref()),
        // An ETA, unlike the rest, does get revised — a new one is newer news.
        eta: ex.eta.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|v| Some(v.to_string())),
        ..Default::default()
    };
    if advanced {
        patch.status = Some(status.to_string());
    }
    let changed = advanced
        || patch.carrier.is_some()
        || patch.tracking_number.is_some()
        || patch.tracking_url.is_some()
        || patch.merchant.is_some()
        || patch.order_ref.is_some()
        || patch.eta.as_ref().is_some_and(|e| *e != existing.eta);

    let shipment = update(pool, user_id, &existing.id, patch).await?.unwrap_or(existing);
    // The event log records every mail, including ones that changed nothing —
    // "we emailed again and said the same thing" is still history.
    add_event(pool, &shipment.id, advanced.then_some(status), &ex.detail, occurred_at, "email").await?;
    let shipment = get(pool, user_id, &shipment.id).await?.unwrap_or(shipment);
    Ok(UpsertResult { shipment, created: false, changed })
}
