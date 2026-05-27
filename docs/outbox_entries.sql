CREATE TABLE outbox_entries (
    id              UUID         PRIMARY KEY,
    status          INTEGER      NOT NULL,
    payload         TEXT         NOT NULL,
    meta            JSONB        NOT NULL,
    scheduled_at    TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    attempts        INTEGER      NOT NULL DEFAULT 0,
    reservation_id  UUID,
    reserved_at     TIMESTAMPTZ,
    received_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    processed_at    TIMESTAMPTZ,
    last_error      TEXT,
    extra_info      JSONB
);

CREATE INDEX idx_outbox_entries_status_scheduled_at
    ON outbox_entries (status, scheduled_at);

CREATE INDEX idx_outbox_entries_status_reserved_at
    ON outbox_entries (status, reserved_at);

-- Optional operational indexes:
-- CREATE INDEX idx_outbox_entries_routing_key
--     ON outbox_entries ((meta ->> 'routing_key'));
--
-- CREATE INDEX idx_outbox_entries_message_id
--     ON outbox_entries ((meta ->> 'message_id'));
