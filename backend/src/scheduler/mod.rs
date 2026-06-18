//! Scheduled agents: user-defined recurring agent runs ("summarize overnight
//! email at 7am"). Each fire runs as a tracked job (`crate::jobs`) over a
//! fresh session — tools enabled; anything gated "ask" parks as a pending
//! approval and the job suspends until decided, never auto-approved. Output
//! lands in the session (visible in History), the Jobs window, the Logs
//! window, and a push notification when FCM is configured.

use std::sync::Arc;

use anyhow::Result;
use chrono::{Datelike, Timelike};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db;
use crate::state::AppState;

/// Worker tick. Schedules are minute-granular, so once a minute is plenty.
const TICK_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledAgent {
    pub id: String,
    pub name: String,
    /// Local time "HH:MM" in the user's home timezone.
    pub time: String,
    /// Days it fires: subset of ["mon".."sun"]. Empty = every day.
    #[serde(default)]
    pub days: Vec<String>,
    /// Provider name; empty = first configured.
    #[serde(default)]
    pub provider: String,
    /// The prompt the agent runs with.
    pub instructions: String,
    pub enabled: bool,
    /// When true the run is "quiet": its automatic completion notification is
    /// suppressed, so the agent only reaches the user if it calls `notify_user`.
    /// Ideal for monitors ("check health; ping me only if something's wrong").
    #[serde(default)]
    pub quiet: bool,
    /// Local date "YYYY-MM-DD" of the last fire — guards double runs.
    #[serde(default)]
    pub last_run: String,
}

fn key(user_id: &str) -> String {
    format!("scheduled_agents:{user_id}")
}

pub async fn get_agents(pool: &SqlitePool, user_id: &str) -> Result<Vec<ScheduledAgent>> {
    Ok(db::settings::get(pool, &key(user_id)).await?.unwrap_or_default())
}

pub async fn set_agents(pool: &SqlitePool, user_id: &str, agents: &[ScheduledAgent]) -> Result<()> {
    db::settings::set(pool, &key(user_id), &agents).await
}

fn weekday_token(weekday: chrono::Weekday) -> &'static str {
    match weekday {
        chrono::Weekday::Mon => "mon",
        chrono::Weekday::Tue => "tue",
        chrono::Weekday::Wed => "wed",
        chrono::Weekday::Thu => "thu",
        chrono::Weekday::Fri => "fri",
        chrono::Weekday::Sat => "sat",
        chrono::Weekday::Sun => "sun",
    }
}

/// Is this agent due at local time `now`? Minute-granular: due once its HH:MM
/// has passed today (catching up after restarts), on a matching day, at most
/// once per local date.
fn due(agent: &ScheduledAgent, now: &chrono::DateTime<chrono_tz::Tz>) -> bool {
    if !agent.enabled {
        return false;
    }
    let Some((h, m)) = agent.time.split_once(':') else { return false };
    let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) else { return false };
    let today = now.format("%Y-%m-%d").to_string();
    if agent.last_run == today {
        return false;
    }
    if !agent.days.is_empty() && !agent.days.iter().any(|d| d == weekday_token(now.weekday())) {
        return false;
    }
    (now.hour(), now.minute()) >= (h, m)
}

pub fn spawn_worker(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(TICK_SECS)).await;
            let users = match db::auth::list_users(&state.db).await {
                Ok(u) => u,
                Err(_) => continue,
            };
            for user in users {
                // Nightly memory consolidation runs independently of scheduled
                // agents, so check it before the early-continue below.
                maybe_consolidate(&state, &user.id).await;

                let mut agents = match get_agents(&state.db, &user.id).await {
                    Ok(a) if !a.is_empty() => a,
                    _ => continue,
                };
                let now = chrono::Utc::now().with_timezone(&state.home_tz(&user.id).await);
                let today = now.format("%Y-%m-%d").to_string();
                let mut changed = false;
                for agent in agents.iter_mut() {
                    if !due(agent, &now) {
                        continue;
                    }
                    // Mark before running so a slow/crashing run can't refire.
                    agent.last_run = today.clone();
                    changed = true;
                    let st = Arc::clone(&state);
                    let uid = user.id.clone();
                    let spec = agent.clone();
                    tokio::spawn(async move {
                        if let Err(e) = run_now(&st, &uid, &spec).await {
                            st.log("scheduler", "error", format!("'{}' failed: {e}", spec.name)).await;
                        }
                    });
                }
                if changed {
                    let _ = set_agents(&state.db, &user.id, &agents).await;
                }
            }
        }
    });
}

/// Local hour (home tz) after which the nightly memory consolidation may run.
const CONSOLIDATE_HOUR: u32 = 3;
/// Skip a nightly run unless at least this many new memories accrued since the
/// last one — no point dreaming when little has changed.
const CONSOLIDATE_MIN_NEW: i64 = 5;

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConsolidationState {
    /// Local date "YYYY-MM-DD" of the last nightly check (run or skipped).
    last_run: String,
    /// Active memory count at the last actual run — gates "enough new".
    last_count: i64,
}

fn consolidation_key(user_id: &str) -> String {
    format!("memory_consolidation_state:{user_id}")
}

/// Once per day after [`CONSOLIDATE_HOUR`], run the memory "dream" for a user —
/// but only when enough new memories have accumulated. Detached so a slow cloud
/// model doesn't stall the scheduler tick.
async fn maybe_consolidate(state: &Arc<AppState>, user_id: &str) {
    let now = chrono::Utc::now().with_timezone(&state.home_tz(user_id).await);
    if now.hour() < CONSOLIDATE_HOUR {
        return;
    }
    let today = now.format("%Y-%m-%d").to_string();
    let mut cs: ConsolidationState =
        db::settings::get(&state.db, &consolidation_key(user_id)).await.ok().flatten().unwrap_or_default();
    if cs.last_run == today {
        return; // already considered today
    }

    let count = db::memories::count_active(&state.db, user_id).await.unwrap_or(0);
    let enough = count - cs.last_count >= CONSOLIDATE_MIN_NEW;
    // Mark today handled regardless (so we don't recheck every tick); only
    // advance the baseline count when we actually run.
    cs.last_run = today;
    if enough {
        cs.last_count = count;
    }
    let _ = db::settings::set(&state.db, &consolidation_key(user_id), &cs).await;
    if !enough {
        return;
    }

    let Some(provider) = crate::memory::consolidate::resolve_provider(state, None).await else {
        return;
    };
    let st = Arc::clone(state);
    let uid = user_id.to_string();
    tokio::spawn(async move {
        st.log("memory", "info", format!("Nightly consolidation starting ({})", provider.name)).await;
        match crate::memory::consolidate::run(&st, &uid, &provider).await {
            Ok(s) => {
                st.log(
                    "memory",
                    "info",
                    format!(
                        "Nightly consolidation: merged {}, dropped {}, lessons {}",
                        s.merged, s.dropped, s.lessons
                    ),
                )
                .await
            }
            Err(e) => st.log("memory", "error", format!("Nightly consolidation failed: {e}")).await,
        }
    });
}

/// Execute one scheduled agent immediately as a tracked job: fresh session,
/// unattended turn; gated tools park as approvals (the job suspends) instead
/// of being skipped. Returns the session id.
pub async fn run_now(state: &Arc<AppState>, user_id: &str, agent: &ScheduledAgent) -> Result<String> {
    let tz = state.home_tz(user_id).await;
    let title = format!("⏰ {} — {}", agent.name, chrono::Utc::now().with_timezone(&tz).format("%-d %b"));
    let session = db::sessions::create(&state.db, user_id, &title).await?;
    db::messages::insert(
        &state.db,
        &session.id,
        "user",
        &serde_json::to_string(&agent.instructions).unwrap_or_default(),
        None,
        None,
    )
    .await?;

    state.log("scheduler", "info", format!("Running '{}'", agent.name)).await;

    // Quiet agents carry a meta flag so the job runner skips the automatic
    // completion notification (they speak up only via notify_user).
    let meta = agent.quiet.then(|| serde_json::json!({ "quiet": true }).to_string());
    let job =
        crate::jobs::start(state, user_id, &session.id, &agent.provider, "scheduled", &agent.name, meta.as_deref())
            .await?;
    crate::jobs::run(Arc::clone(state), job).await;

    Ok(session.id)
}

/// New agent with sane defaults (used by the routes for missing fields).
pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn agent(time: &str, days: Vec<&str>, last_run: &str, enabled: bool) -> ScheduledAgent {
        ScheduledAgent {
            id: "a1".into(),
            name: "t".into(),
            time: time.into(),
            days: days.into_iter().map(String::from).collect(),
            provider: String::new(),
            instructions: "x".into(),
            enabled,
            last_run: last_run.into(),
        }
    }

    fn brisbane(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono_tz::Tz> {
        chrono_tz::Tz::Australia__Brisbane.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn due_after_time_on_matching_day_once_per_day() {
        // 2026-06-05 is a Friday.
        let fri_8am = brisbane(2026, 6, 5, 8, 0);
        assert!(due(&agent("07:00", vec!["fri"], "", true), &fri_8am));
        assert!(!due(&agent("07:00", vec!["fri"], "2026-06-05", true), &fri_8am)); // already ran
        assert!(!due(&agent("09:00", vec!["fri"], "", true), &fri_8am)); // not yet
        assert!(!due(&agent("07:00", vec!["mon"], "", true), &fri_8am)); // wrong day
        assert!(!due(&agent("07:00", vec![], "", false), &fri_8am)); // disabled
        assert!(due(&agent("07:00", vec![], "", true), &fri_8am)); // empty days = daily
    }

    #[test]
    fn due_rejects_malformed_time() {
        let now = brisbane(2026, 6, 5, 8, 0);
        assert!(!due(&agent("late", vec![], "", true), &now));
        assert!(!due(&agent("7", vec![], "", true), &now));
    }
}
