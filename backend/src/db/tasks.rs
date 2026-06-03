use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<String>,
    pub priority: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields that may be changed by an update; `None` leaves the column as-is.
#[derive(Debug, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<Option<String>>,
    pub due_at: Option<Option<String>>,
    pub priority: Option<String>,
    pub status: Option<String>,
}

pub async fn insert(
    pool: &SqlitePool,
    title: &str,
    notes: Option<&str>,
    due_at: Option<&str>,
    priority: &str,
) -> Result<Task> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO tasks (id, title, notes, due_at, priority, status, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'open', ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(notes)
    .bind(due_at)
    .bind(priority)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(Task {
        id,
        title: title.to_string(),
        notes: notes.map(str::to_string),
        due_at: due_at.map(str::to_string),
        priority: priority.to_string(),
        status: "open".to_string(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// List tasks: open before done, then earliest due first (no due date last),
/// then newest created. Optional status filter and title/notes substring.
pub async fn list(
    pool: &SqlitePool,
    status: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> Result<Vec<Task>> {
    let mut sql = String::from(
        "SELECT id, title, notes, due_at, priority, status, created_at, updated_at
         FROM tasks WHERE 1=1",
    );
    if status.is_some() { sql.push_str(" AND status = ?"); }
    if q.is_some()      { sql.push_str(" AND (title LIKE ? OR notes LIKE ?)"); }
    sql.push_str(
        " ORDER BY CASE status WHEN 'open' THEN 0 ELSE 1 END,
                 due_at IS NULL, due_at ASC, created_at DESC LIMIT ?",
    );

    let mut qb = sqlx::query_as::<_, Task>(&sql);
    if let Some(s) = status { qb = qb.bind(s.to_string()); }
    if let Some(s) = q {
        let like = format!("%{s}%");
        qb = qb.bind(like.clone()).bind(like);
    }
    qb = qb.bind(limit);

    Ok(qb.fetch_all(pool).await?)
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Task>> {
    Ok(sqlx::query_as::<_, Task>(
        "SELECT id, title, notes, due_at, priority, status, created_at, updated_at
         FROM tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// Apply a partial update and return the resulting row (None if id unknown).
pub async fn update(pool: &SqlitePool, id: &str, patch: TaskPatch) -> Result<Option<Task>> {
    let Some(mut task) = get(pool, id).await? else {
        return Ok(None);
    };
    if let Some(t) = patch.title { task.title = t; }
    if let Some(n) = patch.notes { task.notes = n; }
    if let Some(d) = patch.due_at { task.due_at = d; }
    if let Some(p) = patch.priority { task.priority = p; }
    if let Some(s) = patch.status { task.status = s; }
    task.updated_at = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE tasks SET title = ?, notes = ?, due_at = ?, priority = ?, status = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&task.title)
    .bind(&task.notes)
    .bind(&task.due_at)
    .bind(&task.priority)
    .bind(&task.status)
    .bind(&task.updated_at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(Some(task))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
