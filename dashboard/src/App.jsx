import {
  Alert,
  Box,
  Button,
  Chip,
  Container,
  Stack,
  Tab,
  Tabs,
  Typography,
} from "@mui/material";
import { Suspense, lazy, useCallback, useEffect, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg?raw";
import { AuthProvider, useAuth } from "./auth/AuthProvider";
import LoginDialog from "./components/auth/LoginDialog";
import { useRuns } from "./hooks/useRuns";
import { useServerStatus } from "./hooks/useServerStatus";
import { useWorkersData } from "./hooks/useWorkersData";
import { asArray } from "./utils/collections";
import { DashboardTheme, ThemeToggle } from "./theme";

const LogsWorkspace = lazy(() => import("./components/LogsWorkspace"));
const PerformanceWorkspace = lazy(() => import("./components/PerformanceWorkspace"));
const RunsWorkspace = lazy(() => import("./components/runs/RunsWorkspace"));
const SettingsWorkspace = lazy(() => import("./components/SettingsWorkspace"));
const WorkersWorkspace = lazy(() => import("./components/WorkersWorkspace"));
const LoadingPanel = ({ label = "Loading..." }) => (
  <Box sx={{ py: 4 }}>
    <Typography variant="body2" color="text.secondary">
      {label}
    </Typography>
  </Box>
);

const DashboardHeader = () => {
  const { authenticated, authEnabled = true, busy, ready, requestLogin, logout } = useAuth();

  return (
    <Box sx={{ mb: 2, display: "flex", flexWrap: "wrap", alignItems: "center", justifyContent: "space-between", gap: 1.5 }}>
      <Box
        role="img"
        aria-label="GammaBoard"
        // Inline only this trusted, bundled SVG so it inherits the theme colours.
        dangerouslySetInnerHTML={{ __html: gammaboardLogo }}
        sx={(theme) => ({ width: { xs: 180, sm: 220 }, maxWidth: "100%",
          color: theme.palette.mode === "dark" ? theme.palette.common.white : theme.palette.common.black,
          "--logo-background": theme.palette.background.default,
          "& svg": { display: "block", width: "100%", height: "auto" } })}
      />
      <Stack direction="row" spacing={1} useFlexGap alignItems="center" sx={{ flexWrap: "wrap" }}>
        <ThemeToggle />
        <Chip
          color={authenticated ? "success" : "default"}
          label={authenticated ? (authEnabled ? "Operator mode" : "Operator mode · no password") : ready ? (authEnabled ? "Read-only" : "Read-only in this browser") : "Checking session"}
          variant={authenticated ? "filled" : "outlined"}
        />
        {authenticated ? (
          <Button onClick={logout} disabled={busy}>
            {authEnabled ? "Log out" : "Switch to read-only"}
          </Button>
        ) : (
          <Button onClick={() => requestLogin()} disabled={!ready || busy}>
            {authEnabled ? "Log in" : "Enable controls"}
          </Button>
        )}
      </Stack>
    </Box>
  );
};

function AppContent() {
  const [showChildRuns, setShowChildRuns] = useState(false);
  const { runs, hasMoreRuns, loadMoreRuns, isLoadingMoreRuns } = useRuns({ includeChildren: showChildRuns });
  const serverStatus = useServerStatus(3000);
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
  const [mode, setMode] = useState("runs");
  const [selectedRun, setSelectedRun] = useState(null);
  const [pendingRunSelection, setPendingRunSelection] = useState(null);
  const runList = asArray(runs);

  useEffect(() => {
    if (runList.length === 0) {
      setSelectedRun(null);
      return;
    }

    if (!selectedRun || !runList.some((run) => run.run_id === selectedRun)) {
      setSelectedRun(runList[0].run_id);
    }
  }, [runList, selectedRun]);

  useEffect(() => {
    if (pendingRunSelection == null) return;
    if (!runList.some((run) => run.run_id === pendingRunSelection)) return;
    setSelectedRun(pendingRunSelection);
    setPendingRunSelection(null);
  }, [pendingRunSelection, runList]);

  const selectRunIncludingChildren = useCallback((runId) => {
    // Navigate when requested, not when a later poll finally contains the run.
    // The user may open another workspace (and an unsaved draft) in between.
    setMode("runs");
    setShowChildRuns(true);
    setPendingRunSelection(runId);
  }, []);

  const selectRun = useCallback((runId) => {
    setPendingRunSelection(null);
    setSelectedRun(runId);
  }, []);

  return (
    <Container maxWidth="xl" sx={{ py: 2 }}>
      <DashboardHeader />
      <LoginDialog />

      <Tabs value={mode} onChange={(_, next) => setMode(next)} sx={{ mb: 2, borderBottom: 1, borderColor: "divider" }}>
        <Tab value="runs" label="Runs" />
        <Tab value="workers" label="Management" />
        <Tab value="performance" label="Performance" />
        <Tab value="logs" label="Logs" />
        <Tab value="settings" label="Settings" />
      </Tabs>

      <Suspense fallback={<LoadingPanel label="Loading workspace..." />}>
        {mode === "runs" ? (
          <RunsWorkspace
            runs={runList}
            selectedRun={selectedRun}
            setSelectedRun={selectRun}
            showChildRuns={showChildRuns}
            setShowChildRuns={setShowChildRuns}
            isConnected={serverStatus.isConnected}
            serverName={serverStatus.serverName}
            onRunCreated={setPendingRunSelection}
            onSelectRun={selectRunIncludingChildren}
            hasMoreRuns={hasMoreRuns}
            loadMoreRuns={loadMoreRuns}
            isLoadingMoreRuns={isLoadingMoreRuns}
          />
        ) : mode === "workers" ? (
          <WorkersWorkspace
            workers={workersData.workers}
            runs={runList}
            isConnected={serverStatus.isConnected}
            lastUpdate={workersData.lastUpdate}
            error={workersData.error}
            serverName={serverStatus.serverName}
          />
        ) : mode === "performance" ? (
          <PerformanceWorkspace
            runs={runList}
            workers={workersData.workers}
            selectedRun={selectedRun}
            setSelectedRun={selectRun}
            showChildRuns={showChildRuns}
            setShowChildRuns={setShowChildRuns}
            isConnected={serverStatus.isConnected}
            serverName={serverStatus.serverName}
            hasMoreRuns={hasMoreRuns}
            loadMoreRuns={loadMoreRuns}
            isLoadingMoreRuns={isLoadingMoreRuns}
          />
        ) : mode === "logs" ? (
          <LogsWorkspace
            runs={runList}
            workers={workersData.workers}
            selectedRun={null}
          />
        ) : (
          <SettingsWorkspace />
        )}
      </Suspense>
    </Container>
  );
}

function SessionGate({ children }) {
  const { ready, sessionError, refreshSession } = useAuth();
  if (!ready || sessionError) {
    return (
      <Container maxWidth="xl" sx={{ py: 3 }}>
        <Typography variant="h5" sx={{ mb: 2 }}>GammaBoard</Typography>
        {!ready ? <LoadingPanel label="Checking browser access..." /> : (
          <Alert severity="error" action={<Button color="inherit" onClick={refreshSession}>Retry</Button>}>
            {sessionError}
          </Alert>
        )}
      </Container>
    );
  }
  return children;
}

function App() {
  return (
    <DashboardTheme>
      <AuthProvider>
        <SessionGate><AppContent /></SessionGate>
      </AuthProvider>
    </DashboardTheme>
  );
}

export default App;
