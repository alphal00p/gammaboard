-- Plot moments are a read model, never a recovery checkpoint. Existing rows
-- stay nullable and are projected on demand; avoid rewriting large histories
-- while migrating a live database.
CREATE FUNCTION gammaboard_plot_value(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE AS $$
DECLARE result JSONB; key TEXT; item JSONB;
BEGIN
    IF jsonb_typeof(value) = 'object' THEN
        result := '{}'::jsonb;
        FOR key,item IN SELECT * FROM jsonb_each(value) LOOP
            IF key = 'diagnostics' THEN CONTINUE; END IF;
            result := result || jsonb_build_object(key, CASE
                WHEN key = 'bundle' THEN '{"histograms":{}}'::jsonb
                WHEN key = 'histograms' THEN CASE WHEN jsonb_typeof(item)='array' THEN '[]'::jsonb ELSE '{}'::jsonb END
                WHEN key = 'discrete_bins' THEN '{}'::jsonb
                WHEN key IN ('max_weighted_positive_sample','max_weighted_negative_sample') THEN 'null'::jsonb
                WHEN key = 'extrema' THEN COALESCE((SELECT jsonb_agg('{"positive":null,"negative":null}'::jsonb) FROM jsonb_array_elements(item)), '[]'::jsonb)
                ELSE gammaboard_plot_value(item) END);
        END LOOP;
        RETURN result;
    ELSIF jsonb_typeof(value) = 'array' THEN
        RETURN COALESCE((SELECT jsonb_agg(gammaboard_plot_value(a.item)) FROM jsonb_array_elements(value) AS a(item)), '[]'::jsonb);
    END IF;
    RETURN value;
END $$;

-- Raster/full-vector/controller payloads do not need this extra read model.
CREATE FUNCTION gammaboard_plot_observable(value JSONB) RETURNS JSONB
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE WHEN value->>'kind' IN ('vector','gammaloop','stratified')
        OR (value ? 'components' AND value ? 'projection')
        OR (value->'estimate' ? 'components')
        THEN gammaboard_plot_value(value) ELSE NULL END
$$;

ALTER TABLE persisted_observable_snapshots ADD COLUMN plot_observable JSONB;
ALTER TABLE runs ADD COLUMN current_plot_observable JSONB;

CREATE FUNCTION gammaboard_store_plot_snapshot() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.plot_observable := gammaboard_plot_observable(NEW.persisted_observable);
    RETURN NEW;
END $$;
CREATE TRIGGER store_plot_snapshot BEFORE INSERT OR UPDATE OF persisted_observable
ON persisted_observable_snapshots FOR EACH ROW EXECUTE FUNCTION gammaboard_store_plot_snapshot();

CREATE FUNCTION gammaboard_store_current_plot() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.current_plot_observable := gammaboard_plot_observable(NEW.current_observable);
    RETURN NEW;
END $$;
CREATE TRIGGER store_current_plot BEFORE INSERT OR UPDATE OF current_observable
ON runs FOR EACH ROW EXECUTE FUNCTION gammaboard_store_current_plot();
