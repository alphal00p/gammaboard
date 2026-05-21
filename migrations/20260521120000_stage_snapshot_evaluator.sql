ALTER TABLE run_stage_snapshots
ADD COLUMN IF NOT EXISTS evaluator JSONB;
