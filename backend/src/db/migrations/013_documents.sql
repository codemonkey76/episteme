-- Documents + RAG: uploaded files are extracted to text, chunked, and each
-- chunk embedded (little-endian f32 blob, same scheme as memories.embedding).
-- status: indexing -> ready, or error (with error_message).

CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'indexing' CHECK (status IN ('indexing', 'ready', 'error')),
    error_message TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_documents_user ON documents(user_id, created_at DESC);

CREATE TABLE document_chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB
);
CREATE INDEX idx_chunks_document ON document_chunks(document_id, seq);
CREATE INDEX idx_chunks_user ON document_chunks(user_id);
