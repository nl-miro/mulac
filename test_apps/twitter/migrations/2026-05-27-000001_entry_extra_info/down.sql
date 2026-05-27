ALTER TABLE outbox_entries
    DROP COLUMN IF EXISTS extra_info;

ALTER TABLE event_entries
    DROP COLUMN IF EXISTS extra_info;

ALTER TABLE command_entries
    DROP COLUMN IF EXISTS extra_info;

ALTER TABLE inbox_entries
    DROP COLUMN IF EXISTS extra_info;
