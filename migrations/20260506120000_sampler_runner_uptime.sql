ALTER TABLE runs
ADD COLUMN IF NOT EXISTS sampler_runner_uptime_ms DOUBLE PRECISION NOT NULL DEFAULT 0.0;

ALTER TABLE runs
DROP CONSTRAINT IF EXISTS runs_sampler_runner_uptime_check;

ALTER TABLE runs
ADD CONSTRAINT runs_sampler_runner_uptime_check CHECK (sampler_runner_uptime_ms >= 0.0);
