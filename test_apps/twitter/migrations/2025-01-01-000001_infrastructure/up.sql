CREATE TABLE inbox_entries (
    id UUID PRIMARY KEY,
    status INT NOT NULL DEFAULT 0,
    payload TEXT NOT NULL,
    meta JSONB NOT NULL,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts INT NOT NULL DEFAULT 0,
    reservation_id UUID,
    reserved_at TIMESTAMPTZ,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ
);
CREATE INDEX idx_inbox_entries_status_scheduled_at ON inbox_entries (status, scheduled_at);
CREATE INDEX idx_inbox_entries_status_reserved_at ON inbox_entries (status, reserved_at);

CREATE TABLE command_entries (
    id UUID PRIMARY KEY,
    command_type TEXT NOT NULL,
    status INT NOT NULL DEFAULT 0,
    payload TEXT NOT NULL,
    meta JSONB,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts INT NOT NULL DEFAULT 0,
    reservation_id UUID,
    reserved_at TIMESTAMPTZ,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ
);
CREATE INDEX idx_command_entries_status_scheduled_at ON command_entries (status, scheduled_at);

CREATE TABLE event_entries (
    id UUID PRIMARY KEY,
    event_type TEXT NOT NULL,
    status INT NOT NULL DEFAULT 0,
    payload TEXT NOT NULL,
    meta JSONB,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts INT NOT NULL DEFAULT 0,
    reservation_id UUID,
    reserved_at TIMESTAMPTZ,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ
);
CREATE INDEX idx_event_entries_status_scheduled_at ON event_entries (status, scheduled_at);

CREATE TABLE outbox_entries (
    id UUID PRIMARY KEY,
    status INT NOT NULL DEFAULT 0,
    payload TEXT NOT NULL,
    meta JSONB NOT NULL,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    attempts INT NOT NULL DEFAULT 0,
    reservation_id UUID,
    reserved_at TIMESTAMPTZ,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ,
    last_error TEXT
);
CREATE INDEX idx_outbox_entries_status_scheduled_at ON outbox_entries (status, scheduled_at);
CREATE INDEX idx_outbox_entries_status_reserved_at ON outbox_entries (status, reserved_at);
