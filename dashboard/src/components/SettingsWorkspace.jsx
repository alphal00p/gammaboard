import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
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
import { fetchSettingsOverview, restartDatabase, shutdownControlProcess } from "../services/api";

const PathValue = ({ label, value }) => (
  <Box>
    <Typography variant="caption" color="text.secondary">
      {label}
    </Typography>
    <Typography
      variant="body2"
      component="div"
      sx={{
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
        overflowWrap: "anywhere",
      }}
    >
      {value || "not set"}
    </Typography>
  </Box>
);

const BoolChip = ({ label, value }) => (
  <Chip
    size="small"
    variant={value ? "filled" : "outlined"}
    color={value ? "success" : "default"}
    label={`${label}: ${value ? "on" : "off"}`}
  />
);

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

const SettingsWorkspace = () => {
  const { authenticated } = useAuth();
  const [settings, setSettings] = useState(null);
  const [error, setError] = useState(null);
  const [restartDbOpen, setRestartDbOpen] = useState(false);
  const [restartingDb, setRestartingDb] = useState(false);
  const [shutdownControlOpen, setShutdownControlOpen] = useState(false);
  const [shuttingDownControl, setShuttingDownControl] = useState(false);
  const [snackbar, setSnackbar] = useState(null);

  useEffect(() => {
    const controller = new AbortController();
    fetchSettingsOverview(controller.signal)
      .then((payload) => {
        setSettings(payload);
        setError(null);
      })
      .catch((err) => {
        if (err?.name !== "AbortError") setError(err?.message || "Failed to load settings.");
      });
    return () => controller.abort();
  }, []);

  if (error) {
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
          <SettingsCard title="Config Files">
            <Stack spacing={1.5}>
              <PathValue label="Runtime config" value={paths.runtime_config} />
              <PathValue label="Server config" value={paths.server_config} />
            </Stack>
          </SettingsCard>
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Resource Paths">
            <Stack spacing={1.5}>
              <PathValue label="Resources root" value={paths.resources_root} />
              <PathValue label="Run templates" value={paths.run_templates_dir} />
              <PathValue label="Task templates" value={paths.task_templates_dir} />
              <PathValue label="Node templates" value={paths.node_templates_dir} />
            </Stack>
          </SettingsCard>
        </Box>
      </Stack>

      <Stack direction={{ xs: "column", md: "row" }} spacing={2}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Runtime">
            <Stack spacing={1.5}>
              <PathValue label="Database URL" value={runtime.database_url} />
              <PathValue label="Postgres data dir" value={paths.postgres_data_dir} />
              <PathValue label="Postgres socket dir" value={paths.postgres_socket_dir} />
              <PathValue label="Postgres log file" value={paths.postgres_log_file} />
              <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
                <BoolChip label="runtime logs" value={runtime.tracing?.persist_runtime_logs} />
                <BoolChip label="wal compression" value={localPostgres.wal_compression} />
                <BoolChip label="sync commit" value={localPostgres.synchronous_commit} />
              </Stack>
            </Stack>
          </SettingsCard>
        </Box>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <SettingsCard title="Server">
            <Stack spacing={1.5}>
              <PathValue label="Name" value={server.name} />
              <PathValue label="API bind" value={server.api_bind} />
              <PathValue label="Allowed origins" value={(server.allowed_origins || []).join(", ")} />
              <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
                <BoolChip label="secure cookie" value={server.secure_cookie} />
                <BoolChip label="auth enabled" value={server.auth_enabled} />
                <BoolChip label="local node spawn" value={server.allow_local_node_spawn} />
              </Stack>
            </Stack>
          </SettingsCard>
        </Box>
      </Stack>

      {authenticated ? (
        <SettingsCard title="Danger Zone">
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
              Kill Control Process
            </Button>
          </Stack>
        </SettingsCard>
      ) : null}

      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
      <Dialog
        open={restartDbOpen}
        onClose={() => (restartingDb ? null : setRestartDbOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Reset Database?</DialogTitle>
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
        open={shutdownControlOpen}
        onClose={() => (shuttingDownControl ? null : setShutdownControlOpen(false))}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Kill Control Process?</DialogTitle>
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
            Kill Control Process
          </Button>
        </DialogActions>
      </Dialog>
    </Stack>
  );
};

export default SettingsWorkspace;
