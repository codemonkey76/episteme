//! Editable system prompts. Every prompt the backend feeds a model lives in
//! this registry with a compiled-in default; the admin can override any of
//! them from the Prompts window (stored in settings under `prompt:<key>`).
//! Defaults stay in code so "reset to default" always works and upgrades can
//! still improve the stock prompts.

use sqlx::SqlitePool;

pub struct PromptDef {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Placeholders substituted at runtime (shown in the editor so edits keep them).
    pub variables: &'static [&'static str],
    pub default: &'static str,
}

pub const PROMPTS: &[PromptDef] = &[
    PromptDef {
        key: "chat_system",
        name: "Chat assistant",
        description: "System message for every chat turn. Sets the assistant's role, \
the user's local date/time, and how to use the calendar/task/note tools.",
        variables: &["{now}", "{timezone}", "{offset}"],
        default: "You are a helpful assistant with access to the user's Microsoft 365 calendar, \
to-do list, and notes.\n\
The current date and time is {now} in the user's timezone ({timezone}, UTC{offset}).\n\
Always present dates and times to the user in this timezone — never in UTC, and \
never show a timezone conversion.\n\
When the user asks to schedule, add an appointment, or set a reminder, call \
create_calendar_event. For to-dos without a fixed appointment time (\"remind me to \
buy milk\", \"I need to renew my rego\"), use the task tools instead. Use the note \
tools to save and recall freeform information the user wants kept (ideas, references, \
details). Resolve relative times (\"tomorrow\", \"next Friday at 3pm\") against the \
current time and output times as RFC3339 with the user's UTC offset ({offset}). For \
reminders, set reminder_minutes_before. After acting, briefly confirm what you did \
in plain language.",
    },
    PromptDef {
        key: "memory_inject",
        name: "Memory preamble",
        description: "Prepended to chat turns when stored memories exist; the memory \
list is appended after this text.",
        variables: &[],
        default: "Persistent memory about the user, learned from past conversations. Use it to \
personalize your responses and stay consistent. Do not mention these notes unless relevant.\n\n\
Memories:\n",
    },
    PromptDef {
        key: "memory_extract",
        name: "Memory extraction",
        description: "Runs after each chat exchange to pull durable facts/preferences \
into memory. Must keep instructing the model to answer with ONLY a JSON array of \
{\"content\", \"category\"} objects.",
        variables: &[],
        default: "You extract durable, long-term memories about the user from a \
single chat exchange. Capture only things worth remembering for FUTURE conversations: stable \
preferences, personal/work facts, ongoing projects, and explicit feedback on how the user wants \
you to behave. Ignore one-off task details, transient context, and anything already obvious.\n\n\
Categorize each as one of: preference, fact, feedback, project, other.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"content\": \"<concise third-person note>\", \"category\": \"<category>\"}. If there is nothing \
worth remembering, return [].",
    },
    PromptDef {
        key: "style_extract",
        name: "Writing-style learning",
        description: "Compares an AI email draft with what the user actually sent and \
extracts writing-style preferences for future drafts. Must keep instructing the model \
to answer with ONLY a JSON array of {\"content\"} objects.",
        variables: &[],
        default: "An AI drafted an email reply for the user; the user edited it and \
sent their own version. Compare the two and extract durable WRITING-STYLE preferences the \
edits reveal — tone, length, formality, greetings and sign-offs, phrasing habits, structure. \
Only extract preferences that would apply to FUTURE emails; ignore changes specific to this \
email's content (names, dates, facts, decisions). State each as a concise instruction for a \
future drafting assistant, e.g. \"Signs off emails with 'Cheers, Shane'\" or \"Prefers replies \
under three sentences\".\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"content\": \"<style note>\"}. If the edits reveal nothing durable, return [].",
    },
    PromptDef {
        key: "commitment_detect",
        name: "Commitment detection",
        description: "Scans emails the user sends for promises (\"I'll do X by Friday\") \
and suggests tasks/events. The current date/time and email body are supplied separately. \
Must keep instructing the model to answer with ONLY a JSON array of \
{\"kind\", \"title\", \"start\", \"end\"} objects.",
        variables: &[],
        default: "The user just SENT the email below. Find commitments THE USER \
made to do something in the future — promises of future action. Ignore commitments made by \
other people, past events, and intentions with no timeframe at all (\"someday\", \"when I \
get a chance\").\n\n\
Classify each commitment:\n\
- \"event\": appointment-like, happens at a specific clock time (e.g. performing maintenance \
at 9pm, attending a meeting). Include \"start\" and, when stated, \"end\".\n\
- \"task\": something to get done within or by a timeframe (e.g. sending a quote by Friday, \
finishing a build this weekend, publishing a video during the week). Include \"start\" as \
the due time.\n\n\
Fuzzy timeframes COUNT as commitments — resolve them to the END of the stated period as the \
due time: \"this weekend\" → Sunday 6pm, \"during the week\" / \"next week\" → Friday 5pm of \
that week, \"by end of month\" → last day of the month 5pm. A named weekday means its NEXT \
occurrence — count forward from today to the first matching weekday. Example: if today is \
Wednesday 10 March, \"by Friday\" means Friday 12 March (two days later), never the Friday \
after that.\n\n\
Times must be RFC3339 with the user's UTC offset, resolved against the current date/time \
given below.\n\n\
Titles must be specific and self-contained: resolve pronouns and vague references (\"this\", \
\"it\", \"that\") using the subject line and the message being replied to. \"I'll get this \
done tonight\" in a thread about cancelling the Jobs server → \"Cancel the Jobs server\", \
never \"Get this done\".\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"kind\": \"task\"|\"event\", \"title\": \"<short imperative description>\", \
\"start\": \"<RFC3339>\"?, \"end\": \"<RFC3339>\"?}. If there are no commitments, return [].",
    },
    PromptDef {
        key: "email_categorizer",
        name: "Email auto-sort",
        description: "Classifies inbox mail for auto-filing. The category names \
(promotions, invoices, notifications, deliveries, attention, none) are wired to \
folders in code — keep them; refine the descriptions of what belongs in each. \
Must keep instructing the model to answer with ONLY a JSON array of \
{\"id\", \"category\"} objects.",
        variables: &[],
        default: "You are an email triage assistant. You are given a \
list of inbox emails. Classify EACH email into exactly one category:\n\
- \"promotions\": marketing, newsletters, sales, offers.\n\
- \"invoices\": bills, receipts, payment confirmations, statements.\n\
- \"notifications\": automated system notifications, alerts, Sentry/error \
reports, CI results, monitoring.\n\
- \"deliveries\": shipping and delivery/order tracking updates.\n\
- \"attention\": anything that needs a human to read or act — personal mail, \
direct questions, requests, anything ambiguous or important.\n\
- \"none\": equivalent to attention; use when unsure. Never guess a low-priority \
category for mail that might matter.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"id\": \"<the email id>\", \"category\": \"<one category>\"}. Include every email \
exactly once.",
    },
    PromptDef {
        key: "email_draft",
        name: "Email reply drafting",
        description: "System message for AI Reply. Learned style preferences are \
appended automatically after this text.",
        variables: &[],
        default: "You draft email replies on behalf of the user. Output only the body of the \
reply — no subject line, no quoted original message, and no placeholder tokens like [Name]. \
If the email contains quoted earlier messages or a reply thread, respond ONLY to the most \
recent message, not to the quoted history. Keep it clear, polite, and concise in a professional \
tone, and directly address anything the email asks.",
    },
    PromptDef {
        key: "email_advise",
        name: "Ask AI about email",
        description: "Default instruction used by \"Ask AI about this email\" when you \
don't type a question. The email itself is appended after it.",
        variables: &[],
        default: "Help me understand this email and tell me what I should do about it.",
    },
];

pub fn def(key: &str) -> Option<&'static PromptDef> {
    PROMPTS.iter().find(|p| p.key == key)
}

fn setting_key(key: &str) -> String {
    format!("prompt:{key}")
}

/// The active text for a prompt: the stored override if one exists and is
/// non-empty, otherwise the compiled-in default. Unknown keys yield "".
pub async fn get(pool: &SqlitePool, key: &str) -> String {
    let stored: Option<String> =
        crate::db::settings::get(pool, &setting_key(key)).await.ok().flatten();
    match stored {
        Some(s) if !s.trim().is_empty() => s,
        _ => def(key).map(|d| d.default.to_string()).unwrap_or_default(),
    }
}

/// The stored override for a prompt, if any (None means "using the default").
pub async fn get_override(pool: &SqlitePool, key: &str) -> Option<String> {
    crate::db::settings::get::<String>(pool, &setting_key(key))
        .await
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
}

pub async fn set_override(pool: &SqlitePool, key: &str, content: &str) -> anyhow::Result<()> {
    crate::db::settings::set(pool, &setting_key(key), &content.to_string()).await
}

pub async fn clear_override(pool: &SqlitePool, key: &str) -> anyhow::Result<()> {
    crate::db::settings::delete(pool, &setting_key(key)).await
}
