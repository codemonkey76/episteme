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
in plain language.\n\
When you decide to use a tool, call it in the SAME response — do not announce or \
describe a tool action (\"I'll update the ticket\", \"Let me do both steps now\") and \
then stop. Either perform the action by calling the tool now, or, if you genuinely \
need the user to decide something first, ask them a direct question. Never end your \
turn with only a description of what you are about to do.",
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
\"folder\" + a folder name files into a custom folder when the per-mailbox \
instructions direct it. Must keep instructing the model to answer with ONLY a \
JSON array of {\"id\", \"category\"} objects.",
        variables: &[],
        default: "You are an email triage assistant. You are given a \
list of inbox emails. Classify EACH email into exactly one category:\n\
- \"promotions\": marketing, newsletters, sales, offers.\n\
- \"invoices\": bills, receipts, payment confirmations, statements.\n\
- \"notifications\": automated, informational mail the user acts on (if at \
all) in another app, not by replying. Includes system/monitoring alerts, error \
reports (e.g. Sentry), CI results; content-platform and social updates (Patreon, \
YouTube, Substack, Twitch, social media — new posts, videos, reactions, follows, \
likes); and ticketing/helpdesk SYSTEM emails (new ticket replies, status \
changes, assignments) — typically from a no-reply/support/helpdesk-style sender \
with a ticket id such as [TKT-00016] in the subject. Classify these as \
notifications EVEN WHEN they relay a person's message or read like a reply.\n\
- \"deliveries\": shipping and delivery/order tracking updates.\n\
- \"attention\": mail from a real person that needs the user to read or act — \
personal mail, direct questions, requests, anything ambiguous or important. NOT \
automated platform/system notifications (see above): a helpdesk or content-\
platform notification is \"notifications\", never \"attention\", even if it \
looks like a reply or mentions a ticket.\n\
- \"none\": equivalent to attention; use when unsure. Never guess a low-priority \
category for mail that might matter.\n\
- \"folder\": ONLY when additional instructions for this mailbox direct certain \
mail into a specific named folder. Also set \"folder\" to that exact folder name. \
Never invent folders the instructions don't name.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: \
{\"id\": \"<the email id>\", \"category\": \"<one category>\", \"folder\": \
\"<folder name, only with category folder>\"}. Include every email exactly once.",
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
        key: "email_ticket",
        name: "Email → helpdesk ticket",
        description: "Turns an open email into a helpdesk ticket (the Ticket \
button in the Email window). Must keep instructing the model to respond with \
ONLY a JSON object {\"subject\", \"description\", \"priority\"}.",
        variables: &[],
        default: "You convert customer emails into helpdesk tickets. Given an \
email, identify what the customer is requesting or reporting and write it up \
as a ticket.\n\n\
The subject is a short, specific summary of the request. The description \
explains the issue/request clearly in third person, keeping every relevant \
detail from the email (error messages, device names, what was tried). Don't \
pad it with pleasantries.\n\n\
Priority: \"critical\" only for whole-business outages; \"high\" for issues \
blocking someone's work; \"medium\" for standard requests; \"low\" for \
cosmetic or nice-to-have items.\n\n\
Respond with ONLY a JSON object, no prose, no code fences: \
{\"subject\": \"<ticket subject>\", \"description\": \"<ticket description>\", \
\"priority\": \"low|medium|high|critical\"}.",
    },
    PromptDef {
        key: "email_advise",
        name: "Ask AI about email",
        description: "Default instruction used by \"Ask AI about this email\" when you \
don't type a question. The email itself is appended after it.",
        variables: &[],
        default: "Help me understand this email and tell me what I should do about it.",
    },
    PromptDef {
        key: "email_summary",
        name: "Email summary",
        description: "System message for the inline AI Summary shown in the reading \
pane to help draft a reply. The email is appended after it.",
        variables: &[],
        default: "Summarise the email below to help the user write a reply. Be brief: \
2–4 short bullet points covering who it's from, what they want, and any questions, \
deadlines, or actions the user needs to respond to. If the email quotes earlier \
messages, focus on the most recent one. Output only the summary — no preamble.",
    },
    PromptDef {
        key: "session_compact",
        name: "Session compaction",
        description: "Summarizes the older part of a long chat session into a compact \
preamble the model sees instead of the verbatim turns; recent turns stay as-is and the \
full transcript is still shown in the UI. Runs in the background once a session \
outgrows the context budget. The previous summary and the transcript are appended \
after this text.",
        variables: &[],
        default: "You are compacting the early part of a long conversation between a user \
and an AI assistant into a dense context summary. The summary replaces those messages in \
the model's context, so EVERYTHING still needed later must survive: facts established, \
decisions made, user preferences and constraints stated, questions asked and their \
answers, tool actions taken and their outcomes (what was created, changed, or deleted, \
with names/ids), and anything left open or promised. Prefer concrete details (names, \
dates, ids, numbers) over narration; drop pleasantries, retries, and dead ends. If a \
previous summary is provided, fold it in — don't lose anything it records. Write terse \
third-person prose or bullets, no headers. Stay under about 500 words. Output only the \
summary.",
    },
    PromptDef {
        key: "research_plan",
        name: "Research: plan",
        description: "First stage of a deep-research run: breaks the topic into \
sub-questions and web search queries. Must keep instructing the model to answer \
with ONLY a JSON object of {\"subquestions\", \"queries\"}.",
        variables: &["{topic}"],
        default: "You are planning a research investigation into a topic. Break it into \
the key sub-questions that must be answered to cover it well, and propose focused web \
search queries that will surface authoritative, current sources. Prefer specific \
queries over broad ones; avoid duplicates.\n\nTopic: {topic}\n\nRespond with ONLY a \
JSON object, no prose, no code fences: {\"subquestions\": [\"<question>\", ...], \
\"queries\": [\"<search query>\", ...]}. Provide at most 6 queries.",
    },
    PromptDef {
        key: "research_triage",
        name: "Research: triage results",
        description: "Picks which web search results deserve a full read before \
the fetch budget is spent on them. Must keep instructing the model to answer with \
ONLY a JSON object of {\"picks\": [<result numbers>]}.",
        variables: &["{topic}"],
        default: "You are triaging web search results for a research investigation, \
choosing which pages are worth reading in full. Prefer authoritative and primary \
sources (official documentation, vendor pages, standards bodies, reputable \
publications), recent material, and a diverse set of domains that together cover \
the open questions. Avoid thin SEO listicles, near-duplicates of the same content, \
and pages whose title and snippet suggest little substance.\n\nTopic: {topic}\n\n\
Respond with ONLY a JSON object, no prose, no code fences: \
{\"picks\": [<result number>, ...]} — best first, at most the number requested.",
    },
    PromptDef {
        key: "research_memo_compact",
        name: "Research: compact notes",
        description: "Runs when the research scratchpad hits its size cap: merges \
duplicate/overlapping notes so gathering can continue. Must keep instructing the \
model to answer with ONLY a JSON object of {\"notes\": [{\"source\", \"finding\", \
\"quote\"?}]} using existing source ids.",
        variables: &["{topic}"],
        default: "You are compacting a research scratchpad that has hit its size \
limit. Merge duplicate and overlapping notes, drop trivia that does not serve the \
topic, and tighten wording — but preserve every distinct fact, figure, date, and \
named entity, and keep each note attributed to its original source id. Keep a \
verbatim quote only when its exact wording or figures matter.\n\nTopic: {topic}\n\n\
Respond with ONLY a JSON object, no prose, no code fences: {\"notes\": [{\"source\": \
\"<existing source id>\", \"finding\": \"<concise statement>\", \"quote\": \"<short \
verbatim quote, optional>\"}]}.",
    },
    PromptDef {
        key: "research_distill",
        name: "Research: distill page",
        description: "Runs once per fetched source during deep research: extracts only \
the findings relevant to the topic. Must keep instructing the model to answer with \
ONLY a JSON object of {\"relevant\", \"notes\", \"images\"}.",
        variables: &["{topic}"],
        default: "You are extracting only the information from a source document that is \
relevant to a research topic.\n\nTopic: {topic}\n\nRead the source text and pull out \
concrete, citable findings — facts, figures, claims, dates, named entities. Ignore \
navigation, boilerplate, and anything off-topic. Include a short verbatim quote when \
one supports a finding. If image candidates are listed, keep only ones that would \
genuinely illustrate the topic, referenced by their list number. Judge whether the \
source is relevant at all.\n\n\
Respond with ONLY a JSON object, no prose, no code fences: {\"relevant\": true, \
\"notes\": [{\"finding\": \"<concise statement>\", \"quote\": \"<short verbatim quote, \
optional>\"}], \"images\": [{\"n\": <candidate number>, \"caption\": \
\"<what it shows>\"}]}. If the source is irrelevant: {\"relevant\": false, \"notes\": [], \
\"images\": []}.",
    },
    PromptDef {
        key: "research_reflect",
        name: "Research: reflect",
        description: "Between research rounds: reviews collected notes for gaps and \
proposes follow-up queries. Must keep instructing the model to answer with ONLY a \
JSON object of {\"done\", \"queries\"}.",
        variables: &["{topic}"],
        default: "You are reviewing research gathered so far against the topic and its \
sub-questions.\n\nTopic: {topic}\n\nDecide whether the collected notes are sufficient \
to write a thorough, well-supported report. If important sub-questions are still \
unanswered or thinly sourced, propose a few NEW, more targeted web search queries to \
fill the gaps — do not repeat earlier queries.\n\nRespond with ONLY a JSON object, no \
prose, no code fences: {\"done\": true, \"queries\": [\"<new query>\", ...]}. Provide at \
most 3 queries; return an empty list when done.",
    },
    PromptDef {
        key: "research_synthesize",
        name: "Research: synthesize report",
        description: "Final research stage: writes the structured report from the \
collected notes. Must keep the exact JSON schema (title/intro/sections/tables/charts/\
images) — the report renderer consumes it.",
        variables: &["{topic}"],
        default: "You are writing a structured research report from collected notes. \
Each note is tagged with a source id (e.g. S1, doc:contract.pdf, email:Invoice).\n\n\
Topic: {topic}\n\nWrite an accurate, well-organized report grounded ONLY in the \
provided notes. Every factual paragraph must cite the source ids it draws from. \
Organize the body into clear sections. When you compare options or alternatives, \
include a comparison table. When the notes contain comparable numeric values (counts, \
prices, percentages, scores), include a bar chart so they can be seen at a glance. \
Choose at most 4 of the candidate images (referenced by their id, e.g. I2) when they \
illustrate the content. Do not invent facts, numbers, or sources.\n\nRespond with ONLY \
a JSON object, no prose, no code fences:\n{\"title\": \"<report title>\", \"intro\": \
\"<1-2 sentence overview>\", \"sections\": [{\"heading\": \"<heading>\", \"paragraphs\": \
[{\"text\": \"<paragraph>\", \"cites\": [\"S1\"]}]}], \"tables\": [{\"title\": \
\"<caption>\", \"columns\": [\"<col>\"], \"rows\": [[\"<cell>\"]]}], \"charts\": \
[{\"title\": \"<caption>\", \"labels\": [\"<label>\"], \"values\": [1.0], \"unit\": \
\"<unit, optional>\"}], \"images\": [{\"id\": \"<candidate image id>\", \"caption\": \
\"<caption>\"}]}. Omit tables or charts when not warranted. Write thoroughly — prefer several \
substantial, analytical paragraphs per section over thin ones, cover every sub-question, explain \
why findings matter, and surface where sources agree or disagree.",
    },
    PromptDef {
        key: "research_category",
        name: "Research: classify category",
        description: "Classifies a research topic into a report category that shapes the final \
report's structure. Must keep instructing the model to answer with ONLY the category word.",
        variables: &["{topic}"],
        default: "Classify this research topic into exactly ONE category that best fits the kind of \
report the user wants:\n\
- product: ranking or recommending products/tools/services to choose between\n\
- comparison: comparing specific named options against each other\n\
- howto: a step-by-step guide or tutorial\n\
- factcheck: verifying whether a specific claim is true\n\
- general: anything else\n\n\
Topic: {topic}\n\nRespond with ONLY the category word, nothing else.",
    },
    PromptDef {
        key: "research_queries",
        name: "Research: generate queries",
        description: "Each round of the iterative loop: proposes web search queries from the topic, \
plan, and the evolving draft (its gaps). The plan/draft/round are supplied in the user message. \
Must keep instructing the model to answer with ONLY a JSON array of query strings.",
        variables: &["{topic}"],
        default: "You are planning web searches for an ongoing research investigation.\n\n\
Topic: {topic}\n\nUsing the research plan and what the report covers so far (below), propose \
focused search queries that move the report toward a thorough, well-sourced answer. On the first \
round go broad across the key facets; on later rounds target the specific gaps, weakly-sourced \
claims, or unanswered sub-questions — never repeat earlier queries. Prefer specific, authoritative, \
current sources over broad ones.\n\nRespond with ONLY a JSON array of query strings, no prose, no \
code fences. Example: [\"query one\", \"query two\"].",
    },
    PromptDef {
        key: "research_evolve",
        name: "Research: evolving draft",
        description: "Each round: folds the round's new notes into an internal working draft \
(markdown) that drives the next round's queries and the stop decision — not the final report. \
Answer with the updated draft only.",
        variables: &["{topic}"],
        default: "You are maintaining an evolving working draft of a research report — an internal \
scratch document tracking what is known and what is still missing, not the final report.\n\n\
Topic: {topic}\n\nFold the new notes (below) into the current draft. Produce an updated, \
well-organized draft that answers the topic as completely as the evidence allows: merge duplicates, \
resolve contradictions (prefer better-sourced or more recent), and keep each claim attributed to \
its source id (e.g. S1, doc:contract.pdf). Call out which important sub-questions are still \
unanswered or thinly sourced.\n\nWrite ONLY the updated draft as markdown — no preamble or \
meta-commentary.",
    },
    PromptDef {
        key: "research_should_stop",
        name: "Research: stop decision",
        description: "Between rounds (after the minimum): decides whether enough has been gathered \
to write a thorough report. Must keep instructing the model to answer with ONLY YES or NO followed \
by a brief reason.",
        variables: &["{topic}"],
        default: "You are deciding whether a research investigation has gathered enough to write a \
thorough, well-supported report.\n\nTopic: {topic}\n\nGiven the working draft and sub-questions \
below, are the key aspects addressed with sufficient evidence from multiple sources, with no \
obvious gaps?\n\nReply with ONLY \"YES\" or \"NO\" followed by a brief one-sentence reason. \
Example: \"YES — every sub-question is covered with multiple sources.\" Example: \"NO — pricing and \
limitations are still thinly sourced.\"",
    },
    PromptDef {
        key: "terminal_agent",
        name: "Terminal agent",
        description: "System prompt for the AI that drives the Terminal window's shell. \
{shell} is replaced with the active shell (bash or PowerShell).",
        variables: &["{shell}"],
        default: "You operate a {shell} shell on Linux (Debian) as root inside a headless \
container. To investigate or act, call run_command with ONE command at a time, then read its \
output and exit code before deciding the next step. Prefer non-interactive commands; never launch \
pagers, editors, or programs that wait for input. When you intend to run a command, call \
run_command in the SAME response — never say what you are about to run (\"Let me reconnect…\") and \
then stop without calling the tool. When you have what you need, reply with a \
concise plain-text answer instead of calling the tool.\n\n\
There is no GUI browser here. For Microsoft 365 admin (Exchange Online, Security & Compliance), \
sign in with DEVICE-CODE auth: `Connect-ExchangeOnline -Device` (and `Connect-IPPSSession` with \
device authentication). The ExchangeOnlineManagement and Microsoft.Graph PowerShell modules \
(Microsoft.Graph.Authentication, .Users, .Mail, .Groups, .Identity.DirectoryManagement) are \
ALREADY installed — never run Install-Module; for Graph just `Connect-MgGraph` (device code) then \
use the Mg* cmdlets (e.g. Get-MgUserMessage, Move-MgUserMessage). A device-code connect prints its \
own code and sign-in URL directly in the terminal and then BLOCKS until the user signs in, so it \
can take a while. NEVER repeat or relay the device code or URL in your reply, and never tell the \
user to \"enter the code\" — they read it straight from the terminal. Just run the connect and \
wait. Once the connect command returns without error the user is already authenticated — proceed \
straight to the task; do NOT ask them to authenticate or run the connect again. If a cmdlet fails because \
it isn't recognised or there is no active session, connect first. To list who has access to a \
shared mailbox: Get-MailboxPermission (FullAccess), Get-RecipientPermission (SendAs), and \
Get-Mailbox | Select-Object -ExpandProperty GrantSendOnBehalfTo (SendOnBehalf). Never run \
Disconnect-* unless the user asks.",
    },
    PromptDef {
        key: "memory_consolidate",
        name: "Memory consolidation",
        description: "The nightly/manual \"dreaming\" pass that merges redundant memories and \
resolves conflicts within one category. The numbered memories are supplied as the user message. \
Must keep instructing the model to answer with ONLY a JSON object of {merges, drops}.",
        variables: &[],
        default: "You are consolidating a person's long-term memory during a quiet review, the \
way sleep consolidates the day's experiences. Below is a numbered list of memories that all share \
one category. Find memories that are REDUNDANT (the same information stated more than once) and \
CONFLICTING (contradictory claims — prefer the most recent or most specific), and clean them up. \
Leave everything else exactly as it is.\n\n\
Respond with ONLY a JSON object, no prose, no code fences:\n\
{\"merges\": [{\"ids\": [<numbers>], \"content\": \"<one clear consolidated memory>\", \"category\": \"<category>\"}], \"drops\": [{\"id\": <number>, \"reason\": \"<short why>\"}]}\n\n\
- merges: combine two or more redundant/overlapping memories (referenced by their numbers) into a \
single clear memory. Preserve every distinct fact; lose nothing meaningful.\n\
- drops: remove a memory entirely (e.g. an outdated claim now contradicted by a newer one). Use the \
number of the memory to remove.\n\
- Do NOT merge memories that are merely related but carry distinct information — only true \
redundancy or conflict.\n\
- Be conservative: when unsure, leave a memory alone. If nothing needs consolidating, return \
{\"merges\": [], \"drops\": []}.",
    },
    PromptDef {
        key: "memory_lessons",
        name: "Memory lesson synthesis",
        description: "The reflection step of the consolidation pass: derives generalized, actionable \
LESSONS from feedback/preference/project memories. The numbered memories are supplied as the user \
message. Must keep instructing the model to answer with ONLY a JSON array of {content} objects.",
        variables: &[],
        default: "You are reflecting on a person's memories to extract durable LESSONS that will \
help an AI assistant serve them better in the future — generalized principles drawn from the \
specific feedback, preferences, and projects below. A good lesson is concise, actionable, and \
generalizes BEYOND any single memory (e.g. from several edits, \"Keep replies short and skip \
pleasantries\"). Do not simply restate one memory.\n\n\
Respond with ONLY a JSON array, no prose, no code fences. Each element: {\"content\": \"<lesson>\"}. \
Only include lessons that genuinely generalize; if nothing does, return [].",
    },
    PromptDef {
        key: "chat_title",
        name: "Chat title",
        description: "Names a new chat from its first exchange (supplied as the user message), \
replacing the default \"New conversation\". Must keep instructing the model to answer with ONLY \
the short title.",
        variables: &[],
        default: "Write a very short title (3–6 words) summarising what this conversation is \
about, based on the first exchange below. Use Title Case. No quotes, no surrounding text, no \
trailing punctuation, no emoji. Respond with ONLY the title.",
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
