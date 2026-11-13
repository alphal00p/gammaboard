-- Presentation terminology only. Raw accumulators, sampler checkpoints, queues,
-- user configuration, histogram names and event statistics keep their formats.
-- Helpers visit known result fields rather than recursively renaming user keys.
CREATE FUNCTION pg_temp.estimator_term(value JSONB) RETURNS JSONB
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT CASE WHEN value = '"combined"'::jsonb THEN '"independent_sum"'::jsonb ELSE value END
$$;

CREATE FUNCTION pg_temp.estimator_measurement(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
BEGIN
    IF jsonb_typeof(value) <> 'object' THEN RETURN value; END IF;
    IF value ? 'basic' THEN
        value := (value - 'basic') || jsonb_build_object('pooled', value->'basic');
    END IF;
    IF value ? 'estimator' THEN
        value := jsonb_set(value, '{estimator}', pg_temp.estimator_term(value->'estimator'));
    END IF;
    RETURN value;
END
$$;

CREATE FUNCTION pg_temp.estimator_measurements(value JSONB) RETURNS JSONB
LANGUAGE sql IMMUTABLE STRICT AS $$
    SELECT CASE WHEN jsonb_typeof(value) = 'array' THEN
        COALESCE((SELECT jsonb_agg(pg_temp.estimator_measurement(item) ORDER BY position)
            FROM jsonb_array_elements(value) WITH ORDINALITY AS entries(item, position)), '[]'::jsonb)
    ELSE value END
$$;

CREATE FUNCTION pg_temp.estimator_output(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
BEGIN
    IF value->>'status' = 'completed' AND jsonb_typeof(value->'results') = 'array' THEN
        value := jsonb_set(value, '{results}', pg_temp.estimator_measurements(value->'results'));
    END IF;
    RETURN value;
END
$$;

CREATE FUNCTION pg_temp.estimator_child(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
BEGIN
    IF value ? 'measurement' THEN
        value := jsonb_set(value, '{measurement}', pg_temp.estimator_output(value->'measurement'));
    END IF;
    IF value ? 'absolute_results' THEN
        value := jsonb_set(value, '{absolute_results}', pg_temp.estimator_measurements(value->'absolute_results'));
    END IF;
    IF value ? 'objective_basic' THEN
        value := (value - 'objective_basic') || jsonb_build_object('objective_pooled', value->'objective_basic');
    END IF;
    IF value ? 'objective_estimator' THEN
        value := jsonb_set(value, '{objective_estimator}', pg_temp.estimator_term(value->'objective_estimator'));
    END IF;
    RETURN value;
END
$$;

CREATE FUNCTION pg_temp.estimator_controller(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE field TEXT;
BEGIN
    FOREACH field IN ARRAY ARRAY['points', 'trials', 'children'] LOOP
        IF jsonb_typeof(value->field) = 'array' THEN
            value := jsonb_set(value, ARRAY[field], COALESCE(
                (SELECT jsonb_agg(pg_temp.estimator_child(item) ORDER BY position)
                 FROM jsonb_array_elements(value->field) WITH ORDINALITY AS entries(item, position)), '[]'::jsonb));
        END IF;
    END LOOP;
    IF value ? 'combined_measurement' THEN
        value := jsonb_set(value, '{combined_measurement}', pg_temp.estimator_output(value->'combined_measurement'));
    END IF;
    RETURN value;
END
$$;

CREATE FUNCTION pg_temp.estimator_result(value JSONB) RETURNS JSONB
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
BEGIN
    -- Only DerivedResultSnapshot has sources + metrics. Raw observable history
    -- is not a measurement payload and must not be altered.
    IF value ? 'sources' AND jsonb_typeof(value->'metrics') = 'array' THEN
        value := jsonb_set(value, '{metrics}', pg_temp.estimator_measurements(value->'metrics'));
        IF value#>>'{observables,estimator}' = 'combined' THEN
            value := jsonb_set(value, '{observables,estimator}', '"independent_sum"'::jsonb);
        END IF;
    END IF;
    RETURN value;
END
$$;

UPDATE run_tasks SET measurement_output = pg_temp.estimator_output(measurement_output)
WHERE measurement_output IS DISTINCT FROM pg_temp.estimator_output(measurement_output);
UPDATE run_tasks SET controller_output = pg_temp.estimator_controller(controller_output)
WHERE controller_output IS DISTINCT FROM pg_temp.estimator_controller(controller_output);
UPDATE persisted_observable_snapshots SET persisted_observable = pg_temp.estimator_result(persisted_observable)
WHERE persisted_observable ? 'sources' AND jsonb_typeof(persisted_observable->'metrics') = 'array'
    AND persisted_observable IS DISTINCT FROM pg_temp.estimator_result(persisted_observable);

DROP FUNCTION pg_temp.estimator_result(JSONB);
DROP FUNCTION pg_temp.estimator_controller(JSONB);
DROP FUNCTION pg_temp.estimator_child(JSONB);
DROP FUNCTION pg_temp.estimator_output(JSONB);
DROP FUNCTION pg_temp.estimator_measurements(JSONB);
DROP FUNCTION pg_temp.estimator_measurement(JSONB);
DROP FUNCTION pg_temp.estimator_term(JSONB);
