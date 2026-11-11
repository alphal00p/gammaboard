import { useWorkspaceActive } from "./common/WorkspaceTabs";
import LoadingStatus from "./common/LoadingStatus";
import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Link,
  Snackbar,
  Stack,
  Typography,
} from "@mui/material";
import { useEffect, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import DisclosureField from "./common/DisclosureField";
import ConfigurationTree from "./panels/ConfigurationTree";
import { fetchSettingsOverview, restartDatabase, shutdownControlProcess } from "../services/api";

const PathValue = ({ label, value }) => <DisclosureField label={label} value={value || "not set"} />;

const SettingsCard = ({ title, children }) => (
  <Card variant="outlined" sx={{ height: "100%" }}>
    <CardContent>
      <Typography variant="h6" sx={{ mb: 2 }}>
        {title}
      </Typography>
      {children}
    </CardContent>
  </Card>
);

const SettingsWorkspace = ({ section = "settings" }) => {
  const active = useWorkspaceActive();
  const { authenticated } = useAuth();
  const [settings, setSettings] = useState(null);
  const [error, setError] = useState(null);
  const [restartDbOpen, setRestartDbOpen] = useState(false);
  const [restartingDb, setRestartingDb] = useState(false);
  const [shutdownControlOpen, setShutdownControlOpen] = useState(false);
  const [shuttingDownControl, setShuttingDownControl] = useState(false);
  const [snackbar, setSnackbar] = useState(null);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    setRefreshing(true);
    fetchSettingsOverview(controller.signal)
      .then((payload) => {
        if (controller.signal.aborted) return;
        setSettings(payload);
        setError(null);
      })
      .catch((err) => {
        if ([401,403,404].includes(err?.status)) setSettings(null);
        if (err?.name !== "AbortError") setError(err?.message || "Failed to load settings.");
      }).finally(() => { if (!controller.signal.aborted) setRefreshing(false); });
    return () => controller.abort();
  }, [active]);

  if (error && !settings) {
    return <Alert severity="error">{error}</Alert>;
  }

  if (!settings) {
    return (
      <Typography variant="body2" color="text.secondary">
        Loading settings...
      </Typography>
    );
  }

  const paths = settings.paths || {};
  const runtime = settings.runtime || {};
  const server = settings.server || {};
  const localPostgres = runtime.local_postgres || {};

  return (
    <Stack spacing={2}>
      <LoadingStatus loading={refreshing} label="Refreshing settings…" />
      {error && <Alert severity="warning">Could not refresh settings. {error}</Alert>}
      <LoadingStatus loading={restartingDb || shuttingDownControl} label={restartingDb ? "Restarting database…" : "Requesting shutdown…"} />
      {section === "settings" && <>
      <SettingsCard title="Repository">
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
          Source and operator documentation live in the project repository.
        </Typography>
        <Link href={settings.repository?.url} target="_blank" rel="noreferrer">
          {settings.repository?.url || "Repository URL unavailable"}
        </Link>
      </SettingsCard>

      <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Config files">
            <Stack spacing={1.5}>
              <PathValue label="Runtime config" value={paths.runtime_config} />
              <PathValue label="Server config" value={paths.server_config} />
            </Stack>
          </SettingsCard>
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Resource paths">
            <Stack spacing={1.5}>
              <PathValue label="Resources root" value={paths.resources_root} />
              <PathValue label="Run templates" value={paths.run_templates_dir} />
              <PathValue label="Task templates" value={paths.task_templates_dir} />
              <PathValue label="Node templates" value={paths.node_templates_dir} />
            </Stack>
          </SettingsCard>
        </Box>
      </Stack>

      </>}
      {section === "database" && <>
      <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Runtime">
            <ConfigurationTree value={{
              database: { url: runtime.database_url },
              postgres: { ...localPostgres, data_dir: paths.postgres_data_dir, socket_dir: paths.postgres_socket_dir, log_file: paths.postgres_log_file },
              tracing: runtime.tracing || {},
            }} />
          </SettingsCard>
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Server">
            <ConfigurationTree value={{ server: { name: server.name, api_bind: server.api_bind,
              allowed_origins: server.allowed_origins || [], secure_cookie: server.secure_cookie,
              auth_enabled: server.auth_enabled, allow_local_node_spawn: server.allow_local_node_spawn } }} />
          </SettingsCard>
        </Box>
      </Stack>

      {authenticated ? (
        <SettingsCard title="Danger zone">
          <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
            These actions affect the whole local control stack, not just one run or node.
          </Typography>
          <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
            <Button color="warning" variant="outlined" disabled={restartingDb} onClick={() => setRestartDbOpen(true)}>
              Reset DB
            </Button>
            <Button
              color="error"
              variant="contained"
              disabled={shuttingDownControl}
              onClick={() => setShutdownControlOpen(true)}
            >
              Kill control process
            </Button>
          </Stack>
        </SettingsCard>
      ) : null}

      </>}

      <Snackbar
        open={active && Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
      <Dialog
        open={active && restartDbOpen}
        onClose={() => (restartingDb ? null : setRestartDbOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Reset database?</DialogTitle>
        <DialogContent>
          This deletes local database state and recreates it from migrations. Running nodes and run execution will be
          interrupted and existing local runs/data will be lost.
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setRestartDbOpen(false)} disabled={restartingDb}>
            Cancel
          </Button>
          <Button
            color="warning"
            variant="contained"
            disabled={restartingDb}
            onClick={async () => {
              setRestartingDb(true);
              try {
                await restartDatabase();
                setRestartDbOpen(false);
                setSnackbar({ message: "Database reset.", severity: "success" });
              } catch (err) {
                setSnackbar({ message: err?.message || "Failed to reset database.", severity: "error" });
              } finally {
                setRestartingDb(false);
              }
            }}
          >
            Confirm Reset
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={active && shutdownControlOpen}
        onClose={() => (shuttingDownControl ? null : setShutdownControlOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Kill control process?</DialogTitle>
        <DialogContent>
          This asks the running backend process to exit. The dashboard and API will go offline until the deployment is
          restarted.
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setShutdownControlOpen(false)} disabled={shuttingDownControl}>
            Cancel
          </Button>
          <Button
            color="error"
            variant="contained"
            disabled={shuttingDownControl}
            onClick={async () => {
              setShuttingDownControl(true);
              try {
                await shutdownControlProcess();
                setShutdownControlOpen(false);
                setSnackbar({ message: "Control shutdown requested.", severity: "success" });
              } catch (err) {
                setSnackbar({ message: err?.message || "Failed to shut down control process.", severity: "error" });
                setShuttingDownControl(false);
              }
            }}
          >
            Kill control process
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
};

export default SettingsWorkspace;
