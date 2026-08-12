-- Shipment/delivery tracking. Rows are created by hand, by the chat agent, or
-- automatically by the email categorizer when it classifies mail as
-- "deliveries" (see categorizer::shipments). Status and ETA are whatever the
-- shipping emails last said — there is no carrier polling; `tracking_url` is
-- the deep link out to the carrier's own live view.

CREATE TABLE IF NOT EXISTS shipments (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    -- What's on the way, in the user's words ("Framework mainboard").
    label TEXT NOT NULL,
    description TEXT,
    carrier TEXT,
    tracking_number TEXT,
    tracking_url TEXT,
    -- Who it's coming from, and their order reference.
    merchant TEXT,
    order_ref TEXT,
    status TEXT NOT NULL DEFAULT 'ordered'
        CHECK (status IN ('ordered', 'in_transit', 'out_for_delivery', 'delivered', 'exception', 'cancelled')),
    eta TEXT,                    -- RFC3339 UTC, nullable
    delivered_at TEXT,
    -- Picture of what's on the way. Held inline (same posture as the rest of
    -- the app: SQLite is the only datastore); never selected by list/get, only
    -- by the dedicated photo route. `photo_mime` non-null ⇒ a photo exists.
    photo BLOB,
    photo_mime TEXT,
    source TEXT NOT NULL DEFAULT 'manual' CHECK (source IN ('manual', 'email', 'agent')),
    -- Graph conversationId of the thread that created it, so later mail in the
    -- same thread updates this shipment instead of forking a new one.
    conversation_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_shipments_user ON shipments(user_id, status, eta);
CREATE INDEX IF NOT EXISTS idx_shipments_conversation ON shipments(user_id, conversation_id);
-- The tracking number is the natural key for matching a follow-up shipping
-- email to an existing shipment; unique per user so the upsert can't fork a
-- duplicate. Partial, because most manual entries have no tracking number yet.
CREATE UNIQUE INDEX IF NOT EXISTS idx_shipments_tracking
    ON shipments(user_id, tracking_number) WHERE tracking_number IS NOT NULL;

-- Append-only history: one row per status update, so the card can show
-- "shipped Tue → out for delivery Thu" rather than only the latest state.
CREATE TABLE IF NOT EXISTS shipment_events (
    id TEXT PRIMARY KEY,
    shipment_id TEXT NOT NULL REFERENCES shipments(id),
    -- The status this update moved the shipment to; null for a note that
    -- didn't change it.
    status TEXT,
    detail TEXT NOT NULL,
    occurred_at TEXT NOT NULL,   -- RFC3339 UTC
    source TEXT NOT NULL DEFAULT 'email',
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_shipment_events_shipment
    ON shipment_events(shipment_id, occurred_at DESC);
