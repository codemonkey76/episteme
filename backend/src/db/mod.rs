use anyhow::Result;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

pub mod auth;
pub mod sessions;
pub mod messages;
pub mod settings;
pub mod pending_actions;
pub mod logs;
pub mod memories;
pub mod tasks;
pub mod notes;
pub mod suggestions;

pub async fn init(url: &str) -> Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await?;

    sqlx::migrate!("src/db/migrations").run(&pool).await?;

    Ok(pool)
}
