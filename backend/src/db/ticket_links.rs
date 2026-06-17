//! Email thread ↔ helpdesk ticket links. See migration 030: an outbound email
//! the agent drafts about a ticket records its Graph `conversationId` here so a
//! later reply in the same thread can be tied back to that ticket.

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

/// A resolved link for an incoming conversation.
#[derive(Debug, Clone)]
pub struct TicketLink {
    pub ticket_id: i64,
    pub integration: Option<String>,
}

/// Record (or refresh) the link from a conversation to a ticket. Idempotent:
/// re-drafting in the same thread just updates the ticket/integration.
pub async fn upsert(
    pool: &SqlitePool,
    user_id: &str,
    conversation_id: &str,
    ticket_id: i64,
    integration: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO ticket_email_links (user_id, conversation_id, ticket_id, integration, created_at) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(user_id, conversation_id) DO UPDATE SET \
           ticket_id = excluded.ticket_id, integration = excluded.integration",
    )
    .bind(user_id)
    .bind(conversation_id)
    .bind(ticket_id)
    .bind(integration)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find the ticket a conversation is linked to, if any.
pub async fn find(
    pool: &SqlitePool,
    user_id: &str,
    conversation_id: &str,
) -> Result<Option<TicketLink>> {
    let row: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT ticket_id, integration FROM ticket_email_links \
         WHERE user_id = ? AND conversation_id = ?",
    )
    .bind(user_id)
    .bind(conversation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(ticket_id, integration)| TicketLink { ticket_id, integration }))
}
