CREATE TABLE run_creation_requests (
    request_id TEXT PRIMARY KEY,
    payload JSONB NOT NULL,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE runs ADD COLUMN archived BOOLEAN NOT NULL DEFAULT false;

CREATE FUNCTION protect_archived_run() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_TABLE_NAME='nodes' THEN
        IF EXISTS(SELECT 1 FROM runs WHERE archived AND id IN (NEW.pool_run_id,NEW.desired_run_id,NEW.active_run_id)) THEN
            RAISE EXCEPTION 'archived result runs are read-only and cannot receive workers';
        END IF;
    ELSIF EXISTS(SELECT 1 FROM runs WHERE id=CASE WHEN TG_OP='DELETE' THEN OLD.run_id ELSE NEW.run_id END AND archived) THEN
        RAISE EXCEPTION 'archived result runs are read-only';
    END IF;
    IF TG_OP='DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER protect_archived_tasks BEFORE INSERT OR UPDATE OR DELETE ON run_tasks
    FOR EACH ROW EXECUTE FUNCTION protect_archived_run();
CREATE TRIGGER protect_archived_assignments BEFORE INSERT OR UPDATE OF pool_run_id,desired_run_id,active_run_id ON nodes
    FOR EACH ROW EXECUTE FUNCTION protect_archived_run();
CREATE TRIGGER protect_archived_batches BEFORE INSERT OR UPDATE ON batches
    FOR EACH ROW EXECUTE FUNCTION protect_archived_run();

CREATE TRIGGER protect_archived_snapshots BEFORE INSERT OR UPDATE OR DELETE ON persisted_observable_snapshots
    FOR EACH ROW EXECUTE FUNCTION protect_archived_run();
