CREATE TABLE inbox_messages (
    id           UUID        PRIMARY KEY,
    message_type TEXT        NOT NULL,
    payload      JSONB       NOT NULL,
    status       TEXT        NOT NULL CHECK (status IN ('received', 'processed', 'failed')),
    received_at  TIMESTAMPTZ NOT NULL,
    processed_at TIMESTAMPTZ,
    error        TEXT
);
CREATE INDEX idx_inbox_messages_status ON inbox_messages (status, received_at);

CREATE TABLE outbox_messages (
    id           UUID    PRIMARY KEY,
    event_type   TEXT    NOT NULL,
    payload      JSONB   NOT NULL,
    status       TEXT    NOT NULL CHECK (status IN ('pending', 'published', 'failed')),
    created_at   TIMESTAMPTZ NOT NULL,
    published_at TIMESTAMPTZ,
    attempts     INT     NOT NULL DEFAULT 0
);
CREATE INDEX idx_outbox_messages_status ON outbox_messages (status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS uq_timelines_user_tweet ON timelines (user_id, tweet_id);
