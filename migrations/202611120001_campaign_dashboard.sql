-- Durable operator overrides, keyed by logical child so planned children inherit them.
CREATE TABLE campaign_queue_tuning_rules (
    id BIGSERIAL PRIMARY KEY,
    campaign_task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    child_label TEXT NOT NULL,
    task_scope TEXT NOT NULL,
    field_name TEXT NOT NULL,
    patch JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(campaign_task_id, child_label, task_scope, field_name)
);

-- Project existing snapshots on demand; never rewrite histories during migration.
-- Keep extrema/diagnostics but exclude histogram bundles and discrete-bin payloads.
CREATE FUNCTION gammaboard_diagnostic_value(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE AS $$
DECLARE result JSONB; key TEXT; item JSONB;
BEGIN
    IF jsonb_typeof(value) = 'object' THEN
        result := '{}'::jsonb;
        FOR key,item IN SELECT * FROM jsonb_each(value) LOOP
            result := result || jsonb_build_object(key, CASE
                WHEN key = 'bundle' THEN '{"histograms":{}}'::jsonb
                WHEN key = 'histograms' THEN CASE WHEN jsonb_typeof(item)='array' THEN COALESCE((SELECT jsonb_agg('{"histograms":{}}'::jsonb) FROM jsonb_array_elements(item)), '[]'::jsonb) ELSE '{}'::jsonb END
                WHEN key = 'discrete_bins' THEN '{}'::jsonb
                ELSE gammaboard_diagnostic_value(item) END);
        END LOOP;
        RETURN result;
    ELSIF jsonb_typeof(value) = 'array' THEN
        RETURN COALESCE((SELECT jsonb_agg(gammaboard_diagnostic_value(a.item)) FROM jsonb_array_elements(value) AS a(item)), '[]'::jsonb);
    END IF;
    RETURN value;
END $$;
