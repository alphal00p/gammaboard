import {
  Alert,
  Box,
  Button,
  Chip,
  Container,
  Stack,
  Typography,
} from "@mui/material";
import { lazy, useCallback, useEffect, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg?raw";
import { AuthProvider, useAuth } from "./auth/AuthProvider";
import LoginDialog from "./components/auth/LoginDialog";
import ConnectionStatus from "./components/ConnectionStatus";
import { useRuns } from "./hooks/useRuns";
import { useServerStatus } from "./hooks/useServerStatus";
import { useWorkersData } from "./hooks/useWorkersData";
import WorkspaceTabs, { WorkspacePanel } from "./components/common/WorkspaceTabs";
import { useDashboardNavigation } from "./hooks/useDashboardNavigation";
import { asArray } from "./utils/collections";
import { DashboardTheme, ThemeToggle } from "./theme";

const LogsWorkspace = lazy(() => import("./components/LogsWorkspace"));
const SystemWorkspace = lazy(() => import("./components/SystemWorkspace"));
const RunsWorkspace = lazy(() => import("./components/runs/RunsWorkspace"));
const LoadingPanel = ({ label = "Loading..." }) => (
  <Box sx={{ py: 4 }}>
    <Typography variant="body2" color="text.secondary">
      {label}
    </Typography>
  </Box>
);

const DashboardHeader = ({ serverStatus }) => {
  const { authenticated, authEnabled = true, busy, ready, requestLogin, logout } = useAuth();

  return (
    <Box component="header" sx={{ mb: 1, display: "grid", alignItems: "center", columnGap: 2, rowGap: 0.5,
      gridTemplateColumns: { xs: "minmax(0, 1fr) minmax(0, 1fr)", sm: "180px minmax(0, 1fr)", lg: "180px minmax(0, 1fr) auto" },
      gridTemplateAreas: { xs: '"logo connection" "auth auth"', sm: '"logo connection" "logo auth"', lg: '"logo connection auth"' } }}>
      <Box
        role="img"
        aria-label="GammaBoard"
        // Inline only this trusted, bundled SVG so it inherits the theme colours.
        dangerouslySetInnerHTML={{ __html: gammaboardLogo }}
        sx={(theme) => ({ gridArea: "logo", width: { xs: 150, sm: 180 }, maxWidth: "100%",
          color: theme.palette.mode === "dark" ? theme.palette.common.white : theme.palette.common.black,
          "--logo-background": theme.palette.background.default,
          "& svg": { display: "block", width: "100%", height: "auto" } })}
      />
      <Box sx={{ gridArea: "connection", minWidth: 0 }}>
        <ConnectionStatus {...serverStatus} />
      </Box>
      <Stack direction="row" spacing={1} useFlexGap alignItems="center" sx={{ gridArea: "auth", flexWrap: "wrap", justifyContent: "flex-end", minWidth: 0 }}>
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
  const [navigation, navigate] = useDashboardNavigation();
  const { workspace: mode, runId: selectedRun, children: showChildRuns } = navigation;
  const setShowChildRuns = (value) => navigate({ children: value });
  const { runs, hasMoreRuns, loadMoreRuns, isLoadingMoreRuns, isLoadingRuns, selectedRunMissing } = useRuns({ includeChildren: showChildRuns, selectedRun });
  const serverStatus = useServerStatus(3000);
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
  const [pendingRunSelection, setPendingRunSelection] = useState(null);
  const runList = asArray(runs);

  useEffect(() => {
    if (isLoadingRuns) return;
    if (runList.length === 0) {
      if (selectedRun != null && selectedRunMissing) navigate({ runId: null, taskId: null }, { replace: true });
      return;
    }

    if (!selectedRun || !runList.some((run) => run.run_id === selectedRun)) {
      if (selectedRun && !selectedRunMissing) return;
      navigate({ runId: runList[0].run_id, taskId: null }, { replace: true });
    }
  }, [runList, selectedRun, isLoadingRuns, selectedRunMissing, navigate]);

  useEffect(() => {
    if (pendingRunSelection == null) return;
    if (!runList.some((run) => run.run_id === pendingRunSelection)) return;
    navigate({ runId: pendingRunSelection, taskId: null });
    setPendingRunSelection(null);
  }, [pendingRunSelection, runList, navigate]);

  const selectRunIncludingChildren = useCallback((runId) => {
    // Navigate when requested, not when a later poll finally contains the run.
    // The user may open another workspace (and an unsaved draft) in between.
    navigate({ workspace: "runs", runTab: "tasks", taskId: null, children: true });
    setPendingRunSelection(runId);
  }, [navigate]);

  const selectRun = useCallback((runId) => {
    if (runId === selectedRun) return;
    setPendingRunSelection(null);
    navigate({ runId, taskId: null });
  }, [navigate, selectedRun]);

  return (
    <Container maxWidth="xl" sx={{ py: 2 }}>
      <DashboardHeader serverStatus={serverStatus} />
      <LoginDialog />

      <Box sx={{ position: "sticky", top: 0, zIndex: (theme) => theme.zIndex.appBar, bgcolor: "background.default", pt: 1, mb: 2 }}>
        <WorkspaceTabs id="main" label="Main navigation" value={mode} onChange={(workspace) => navigate({ workspace })}
          level="primary" tabs={[["runs", "Runs"], ["system", "System"], ["logs", "Logs"]]} />
      </Box>
      <WorkspacePanel id="main" value="runs" active={mode === "runs"}>
        <RunsWorkspace runs={runList} workers={workersData.workers} selectedRun={selectedRun} setSelectedRun={selectRun}
          navigation={navigation} onNavigate={navigate}
          showChildRuns={showChildRuns} setShowChildRuns={setShowChildRuns}
          isConnected={serverStatus.isConnected}
          onRunCreated={setPendingRunSelection} onSelectRun={selectRunIncludingChildren}
          hasMoreRuns={hasMoreRuns} loadMoreRuns={loadMoreRuns} isLoadingMoreRuns={isLoadingMoreRuns} isLoadingRuns={isLoadingRuns} />
      </WorkspacePanel>
      <WorkspacePanel id="main" value="system" active={mode === "system"}>
        <SystemWorkspace view={navigation.systemTab} onViewChange={(systemTab) => navigate({ systemTab })}
          workers={workersData.workers} isLoading={workersData.isLoading} runs={runList}
          isConnected={serverStatus.isConnected} error={workersData.error} />
      </WorkspacePanel>
      <WorkspacePanel id="main" value="logs" active={mode === "logs"}>
        <LogsWorkspace runs={runList} workers={workersData.workers} selectedRun={null} />
      </WorkspacePanel>
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
