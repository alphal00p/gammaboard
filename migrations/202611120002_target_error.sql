-- Optional component uncertainties for reference values; omitted errors are zero.
ALTER TABLE runs ADD COLUMN target_error JSONB;
