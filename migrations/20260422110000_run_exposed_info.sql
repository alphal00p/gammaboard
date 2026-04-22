ALTER TABLE runs
    ADD COLUMN exposed_info JSONB NOT NULL DEFAULT '{}'::jsonb;
