-- Conversation search: FTS5 index over user/assistant message text, kept in
-- sync by triggers. Stored content is JSON-encoded (a quoted string, or a
-- multimodal object whose searchable part is $.text) — the CASE extracts the
-- plain text so the index never sees JSON escapes or base64 image payloads.

CREATE VIRTUAL TABLE message_fts USING fts5(
    message_id UNINDEXED,
    session_id UNINDEXED,
    text
);

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages
WHEN new.role IN ('user', 'assistant')
BEGIN
    INSERT INTO message_fts (message_id, session_id, text)
    VALUES (
        new.id,
        new.session_id,
        CASE
            WHEN json_valid(new.content) AND json_type(new.content) = 'text'
                THEN json_extract(new.content, '$')
            WHEN json_valid(new.content) AND json_type(new.content) = 'object'
                THEN coalesce(json_extract(new.content, '$.text'), '')
            ELSE new.content
        END
    );
END;

-- Session deletes cascade to messages; this fires per deleted row.
CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages
BEGIN
    DELETE FROM message_fts WHERE message_id = old.id;
END;

-- Backfill existing history.
INSERT INTO message_fts (message_id, session_id, text)
SELECT
    id,
    session_id,
    CASE
        WHEN json_valid(content) AND json_type(content) = 'text'
            THEN json_extract(content, '$')
        WHEN json_valid(content) AND json_type(content) = 'object'
            THEN coalesce(json_extract(content, '$.text'), '')
        ELSE content
    END
FROM messages
WHERE role IN ('user', 'assistant');
