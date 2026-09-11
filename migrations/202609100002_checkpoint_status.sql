ALTER TABLE runs ADD COLUMN checkpoint_status JSONB NOT NULL DEFAULT '{}'::jsonb;
