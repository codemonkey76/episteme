-- Links an email thread (Microsoft Graph conversationId) to a helpdesk ticket.
-- Written when the agent drafts an email about a ticket (e.g. lodging a fault
-- with an upstream provider); read by the categorizer so a later reply in the
-- same thread can be recognised as an update to that ticket and surfaced as an
-- actionable notification rather than sorted away as ordinary mail.
CREATE TABLE IF NOT EXISTS ticket_email_links (
    user_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    ticket_id INTEGER NOT NULL,
    integration TEXT,                       -- named helpdesk instance (null = default/sole)
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, conversation_id)
);
