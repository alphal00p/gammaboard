-- Activity and desired-assignment updates do not change allocated worker time.
-- Charging them also makes a bulk pause lock the shared task row between node
-- rows, in the opposite order to concurrent worker heartbeats.
DROP TRIGGER nodes_account_cpu_time ON nodes;
CREATE TRIGGER nodes_account_cpu_time
BEFORE UPDATE OF lease_expires_at, active_run_id, active_role, capabilities ON nodes
FOR EACH ROW
EXECUTE FUNCTION account_node_cpu_time();
