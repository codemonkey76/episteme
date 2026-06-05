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
    /// To-do list this task belongs to; None = the implicit "General" list.
    pub list_id: Option<String>,
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
    /// Some(None) moves the task back to the implicit General list.
    pub list_id: Option<Option<String>>,
}

/// Which list to read tasks from: everything, the implicit General list
/// (list_id IS NULL), or one specific named list.
#[derive(Debug, Clone, Copy)]
pub enum ListFilter<'a> {
    All,
    General,
    List(&'a str),
}

pub async fn insert(
    pool: &SqlitePool,
    user_id: &str,
    title: &str,
    notes: Option<&str>,
    due_at: Option<&str>,
    priority: &str,
    list_id: Option<&str>,
) -> Result<Task> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO tasks (id, user_id, title, notes, due_at, priority, status, list_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(title)
    .bind(notes)
    .bind(due_at)
    .bind(priority)
    .bind(list_id)
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
        list_id: list_id.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// List tasks: open before done, then earliest due first (no due date last),
/// then newest created. Optional status filter, title/notes substring, and
/// list filter.
pub async fn list(
    pool: &SqlitePool,
    user_id: &str,
    status: Option<&str>,
    q: Option<&str>,
    list: ListFilter<'_>,
    limit: i64,
) -> Result<Vec<Task>> {
    let mut sql = String::from(
        "SELECT id, title, notes, due_at, priority, status, list_id, created_at, updated_at
         FROM tasks WHERE user_id = ?",
    );
    if status.is_some() { sql.push_str(" AND status = ?"); }
    if q.is_some()      { sql.push_str(" AND (title LIKE ? OR notes LIKE ?)"); }
    match list {
        ListFilter::All => {}
        ListFilter::General => sql.push_str(" AND list_id IS NULL"),
        ListFilter::List(_) => sql.push_str(" AND list_id = ?"),
    }
    sql.push_str(
        " ORDER BY CASE status WHEN 'open' THEN 0 ELSE 1 END,
                 due_at IS NULL, due_at ASC, created_at DESC LIMIT ?",
    );

    let mut qb = sqlx::query_as::<_, Task>(&sql);
    qb = qb.bind(user_id);
    if let Some(s) = status { qb = qb.bind(s.to_string()); }
    if let Some(s) = q {
        let like = format!("%{s}%");
        qb = qb.bind(like.clone()).bind(like);
    }
    if let ListFilter::List(id) = list { qb = qb.bind(id.to_string()); }
    qb = qb.bind(limit);

    Ok(qb.fetch_all(pool).await?)
}

pub async fn get(pool: &SqlitePool, user_id: &str, id: &str) -> Result<Option<Task>> {
    Ok(sqlx::query_as::<_, Task>(
        "SELECT id, title, notes, due_at, priority, status, list_id, created_at, updated_at
         FROM tasks WHERE id = ? AND user_id = ?",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

/// Apply a partial update and return the resulting row (None if id unknown).
pub async fn update(
    pool: &SqlitePool,
    user_id: &str,
    id: &str,
    patch: TaskPatch,
) -> Result<Option<Task>> {
    let Some(mut task) = get(pool, user_id, id).await? else {
        return Ok(None);
    };
    if let Some(t) = patch.title { task.title = t; }
    if let Some(n) = patch.notes { task.notes = n; }
    if let Some(d) = patch.due_at { task.due_at = d; }
    if let Some(p) = patch.priority { task.priority = p; }
    if let Some(s) = patch.status { task.status = s; }
    if let Some(l) = patch.list_id { task.list_id = l; }
    task.updated_at = Utc::now().to_rfc3339();

    sqlx::query(
        "UPDATE tasks SET title = ?, notes = ?, due_at = ?, priority = ?, status = ?, list_id = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(&task.title)
    .bind(&task.notes)
    .bind(&task.due_at)
    .bind(&task.priority)
    .bind(&task.status)
    .bind(&task.list_id)
    .bind(&task.updated_at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(Some(task))
}

pub async fn delete(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM tasks WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── To-do lists ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskList {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

pub async fn lists(pool: &SqlitePool, user_id: &str) -> Result<Vec<TaskList>> {
    Ok(sqlx::query_as::<_, TaskList>(
        "SELECT id, name, created_at FROM task_lists WHERE user_id = ? ORDER BY name COLLATE NOCASE",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Find a list by (case-insensitive) name.
pub async fn list_by_name(pool: &SqlitePool, user_id: &str, name: &str) -> Result<Option<TaskList>> {
    Ok(sqlx::query_as::<_, TaskList>(
        "SELECT id, name, created_at FROM task_lists WHERE user_id = ? AND name = ? COLLATE NOCASE",
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(pool)
    .await?)
}

pub async fn insert_list(pool: &SqlitePool, user_id: &str, name: &str) -> Result<TaskList> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO task_lists (id, user_id, name, created_at) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(user_id)
        .bind(name)
        .bind(&now)
        .execute(pool)
        .await?;
    Ok(TaskList { id, name: name.to_string(), created_at: now })
}

pub async fn rename_list(pool: &SqlitePool, user_id: &str, id: &str, name: &str) -> Result<bool> {
    let res = sqlx::query("UPDATE task_lists SET name = ? WHERE id = ? AND user_id = ?")
        .bind(name)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Delete a list; its tasks fall back to the implicit General list.
pub async fn delete_list(pool: &SqlitePool, user_id: &str, id: &str) -> Result<()> {
    sqlx::query("UPDATE tasks SET list_id = NULL WHERE list_id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM task_lists WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}
