-- Optional structured payload for actionable notifications. Carries the data an
-- action button needs without extra round-trips — e.g. a ticket_update
-- notification stores {ticket_id, integration, draft_reply} so "Review & send
-- reply" can open a pre-drafted helpdesk reply. NULL for ordinary notifications.
ALTER TABLE notifications ADD COLUMN data TEXT;
