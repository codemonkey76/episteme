//! Resolve which integration instance a tool should use. Instances are named
//! rows in the `integrations` table (see `db::integrations`); a tool may pass an
//! explicit instance name, otherwise we fall back to the type's default, then to
//! the sole instance, and otherwise error asking the caller to choose.

use anyhow::{anyhow, Result};
use sqlx::SqlitePool;

use crate::db;
use crate::db::integrations::Integration;

/// Human label for a kind, used in error/preamble text.
pub fn label(kind: &str) -> &str {
    match kind {
        "helpdesk" => "Helpdesk",
        "phoneus" => "PhoneUs",
        "github" => "GitHub",
        "recipes" => "Recipe Box",
        other => other,
    }
}

fn names_str(rows: &[Integration]) -> String {
    rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ")
}

/// Resolve the instance for `kind`. Precedence: explicit name/id → type default
/// → sole instance → error listing the options.
pub async fn resolve(
    pool: &SqlitePool,
    user_id: &str,
    kind: &str,
    instance: Option<&str>,
) -> Result<Integration> {
    // Lazily fold any legacy single-config into a row (idempotent, cheap).
    let _ = db::integrations::import_legacy(pool, user_id).await;

    let rows = db::integrations::list_by_kind(pool, user_id, kind).await?;
    if rows.is_empty() {
        return Err(anyhow!("{} not connected — add it in Settings → Integrations", label(kind)));
    }

    if let Some(sel) = instance.map(str::trim).filter(|s| !s.is_empty()) {
        return rows
            .iter()
            .find(|r| r.id == sel || r.name.eq_ignore_ascii_case(sel))
            .cloned()
            .ok_or_else(|| {
                anyhow!("no {} instance named '{sel}' — options: {}", label(kind), names_str(&rows))
            });
    }

    if rows.len() == 1 {
        return Ok(rows.into_iter().next().unwrap());
    }
    if let Some(def) = rows.iter().find(|r| r.is_default) {
        return Ok(def.clone());
    }
    Err(anyhow!(
        "multiple {} instances are configured ({}). Ask the user which to use and pass it as `integration`.",
        label(kind),
        names_str(&rows)
    ))
}

/// Configured instance names for `kind` (for the system preamble).
pub async fn names(pool: &SqlitePool, user_id: &str, kind: &str) -> Vec<String> {
    db::integrations::list_by_kind(pool, user_id, kind)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn add(pool: &SqlitePool, kind: &str, name: &str, default: bool) {
        db::integrations::insert(pool, "u1", kind, name, default, "{}").await.unwrap();
    }

    #[tokio::test]
    async fn resolve_precedence() {
        let pool = db::init("sqlite::memory:").await.unwrap();

        // No instances → error.
        assert!(resolve(&pool, "u1", "phoneus", None).await.is_err());

        // Sole instance is used without naming it.
        add(&pool, "phoneus", "ASG", false).await;
        assert_eq!(resolve(&pool, "u1", "phoneus", None).await.unwrap().name, "ASG");

        // Two instances, none default, unspecified → ambiguous error.
        add(&pool, "phoneus", "Acme", false).await;
        assert!(resolve(&pool, "u1", "phoneus", None).await.is_err());

        // Explicit name wins (case-insensitive).
        assert_eq!(resolve(&pool, "u1", "phoneus", Some("acme")).await.unwrap().name, "Acme");

        // Unknown name → error.
        assert!(resolve(&pool, "u1", "phoneus", Some("Nope")).await.is_err());

        // A default is used when unspecified.
        let acme = resolve(&pool, "u1", "phoneus", Some("Acme")).await.unwrap();
        db::integrations::set_default(&pool, "u1", &acme.id).await.unwrap();
        assert_eq!(resolve(&pool, "u1", "phoneus", None).await.unwrap().name, "Acme");
    }
}
