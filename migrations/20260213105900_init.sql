-- Add migration script here
CREATE TABLE IF NOT EXISTS runs (
    id SERIAL PRIMARY KEY,
    started_at TIMESTAMPTZ DEFAULT now(),
    parameters JSONB
);


CREATE TABLE IF NOT EXISTS results (
    id BIGSERIAL PRIMARY KEY,
    run_id INT REFERENCES runs(id),
    step INT,
    value DOUBLE PRECISION,
    created_at TIMESTAMPTZ DEFAULT now()
);
