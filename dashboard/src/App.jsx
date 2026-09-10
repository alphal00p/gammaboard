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
import gammaboardLogo from "./assets/gammalooplogo.svg";
import { AuthProvider, useAuth } from "./auth/AuthProvider";
import LoginDialog from "./components/auth/LoginDialog";
import { useRuns } from "./hooks/useRuns";
import { useServerStatus } from "./hooks/useServerStatus";
import { useWorkersData } from "./hooks/useWorkersData";
import { asArray } from "./utils/collections";

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
  const { authenticated, busy, ready, requestLogin, logout } = useAuth();

  return (
    <Box sx={{ mb: 3, display: "flex", flexWrap: "wrap", justifyContent: "space-between", gap: 2 }}>
      <Box>
        <Box
          component="img"
          src={gammaboardLogo}
          alt="GammaBoard"
          sx={{ display: "block", width: "min(100%, 320px)", height: "auto", mb: 1 }}
        />
        <Typography variant="body2" color="text.secondary">
          Real-time Monte Carlo simulation monitoring
        </Typography>
      </Box>
      <Stack direction="row" spacing={1} alignItems="center">
        <Chip
          color={authenticated ? "success" : "default"}
          label={authenticated ? "Operator mode" : ready ? "Read-only" : "Checking session"}
          variant={authenticated ? "filled" : "outlined"}
        />
        {authenticated ? (
          <Button onClick={logout} disabled={busy}>
            Log Out
          </Button>
        ) : (
          <Button onClick={() => requestLogin()} disabled={!ready || busy}>
            Log In
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
    setMode("runs");
    setPendingRunSelection(null);
  }, [pendingRunSelection, runList]);

  const selectRunIncludingChildren = useCallback((runId) => {
    setShowChildRuns(true);
    setPendingRunSelection(runId);
  }, []);

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <DashboardHeader />
      <LoginDialog />

      <Tabs value={mode} onChange={(_, next) => setMode(next)} sx={{ mb: 3 }}>
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
            setSelectedRun={setSelectedRun}
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
            setSelectedRun={setSelectedRun}
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
    <AuthProvider>
      <SessionGate><AppContent /></SessionGate>
    </AuthProvider>
  );
}

export default App;
