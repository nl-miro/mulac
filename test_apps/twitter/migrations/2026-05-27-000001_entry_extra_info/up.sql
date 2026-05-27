ALTER TABLE inbox_entries
    ADD COLUMN IF NOT EXISTS extra_info JSONB;

ALTER TABLE command_entries
    ADD COLUMN IF NOT EXISTS extra_info JSONB;

ALTER TABLE event_entries
    ADD COLUMN IF NOT EXISTS extra_info JSONB;

ALTER TABLE outbox_entries
    ADD COLUMN IF NOT EXISTS extra_info JSONB;
