import { Alert, Box, Button, Chip, Container, Snackbar, Stack, Tab, Tabs, TextField, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg";
import { AuthProvider, useAuth } from "./auth/AuthProvider";
import EvaluatorPanel from "./components/EvaluatorPanel";
import LogsWorkspace from "./components/LogsWorkspace";
import PerformanceWorkspace from "./components/PerformanceWorkspace";
import RunInfo from "./components/RunInfo";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import TaskOutputPanel from "./components/TaskOutputPanel";
import TaskQueuePanel from "./components/TaskQueuePanel";
import WorkersWorkspace from "./components/WorkersWorkspace";
import LoginDialog from "./components/auth/LoginDialog";
import RunScopedWorkspace from "./components/common/RunScopedWorkspace";
import { useRunConfigPanels } from "./hooks/useRunConfigPanels";
import { useRuns } from "./hooks/useRuns";
import { useRunTasks } from "./hooks/useRunTasks";
import { useWorkersData } from "./hooks/useWorkersData";
import { autoAssignRun, pauseRun } from "./services/api";
import { asArray } from "./utils/collections";
import { asTaskList, getCurrentTask } from "./utils/tasks";

const DashboardHeader = () => {
  const { authenticated, busy, ready, requestLogin, logout } = useAuth();

  return (
    <Box sx={{ mb: 3, display: "flex", flexWrap: "wrap", justifyContent: "space-between", gap: 2 }}>
      <Box>
        <Box
          component="img"
          src={gammaboardLogo}
          alt="Gammaboard"
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

const RunModeContent = ({ runs, selectedRun }) => {
  const currentRun = runs.find((entry) => entry.run_id === selectedRun);
  const { tasks } = useRunTasks(selectedRun, 2000);
  const { evaluator, sampler } = useRunConfigPanels({ runId: selectedRun, pollMs: 5000 });
  const [selectedTaskId, setSelectedTaskId] = useState(null);
  const [snackbar, setSnackbar] = useState(null);
  const [pausing, setPausing] = useState(false);
  const [autoAssigning, setAutoAssigning] = useState(false);
  const [maxEvaluators, setMaxEvaluators] = useState("");
  const { authenticated } = useAuth();

  useEffect(() => {
    const taskList = asTaskList(tasks);
    if (taskList.length === 0) {
      setSelectedTaskId(null);
      return;
    }
    if (selectedTaskId != null && taskList.some((task) => task.id === selectedTaskId)) {
      return;
    }
    setSelectedTaskId(getCurrentTask(taskList)?.id ?? taskList[0].id ?? null);
  }, [selectedTaskId, tasks]);

  if (!currentRun) {
    return (
      <Alert severity="warning" sx={{ mb: 3 }}>
        Selected run not found in current run list.
      </Alert>
    );
  }

  const taskList = asTaskList(tasks);
  const selectedTask = taskList.find((task) => task.id === selectedTaskId) ?? getCurrentTask(taskList) ?? null;

  return (
    <>
      {authenticated ? (
        <Box sx={{ mb: 2, display: "flex", justifyContent: "flex-end" }}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
            <TextField
              size="small"
              label="Max Evaluators"
              value={maxEvaluators}
              onChange={(event) => setMaxEvaluators(event.target.value.replace(/[^\d]/g, ""))}
              sx={{ minWidth: 160 }}
            />
            <Button
              variant="contained"
              disabled={!selectedRun || pausing || autoAssigning}
              onClick={async () => {
                setAutoAssigning(true);
                try {
                  const limit = maxEvaluators.trim() ? Number(maxEvaluators) : null;
                  const response = await autoAssignRun(selectedRun, { maxEvaluators: limit });
                  const assignedEvaluators = Array.isArray(response?.assigned_evaluators)
                    ? response.assigned_evaluators.length
                    : 0;
                  const assignedSampler = response?.assigned_sampler ? 1 : 0;
                  setSnackbar({
                    message: `Auto-assign updated ${assignedSampler + assignedEvaluators} node(s).`,
                    severity: "success",
                  });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to auto-assign run.", severity: "error" });
                } finally {
                  setAutoAssigning(false);
                }
              }}
            >
              Auto-Assign
            </Button>
            <Button
              variant="contained"
              color="warning"
              disabled={!selectedRun || pausing || autoAssigning}
              onClick={async () => {
                setPausing(true);
                try {
                  await pauseRun(selectedRun);
                  setSnackbar({ message: "Pause requested.", severity: "success" });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to pause run.", severity: "error" });
                } finally {
                  setPausing(false);
                }
              }}
            >
              Pause Run
            </Button>
          </Stack>
        </Box>
      ) : null}
      <RunInfo runId={selectedRun} />
      <TaskQueuePanel tasks={taskList} selectedTaskId={selectedTask?.id ?? null} onSelectTask={setSelectedTaskId} />
      <EvaluatorPanel run={currentRun} panelResponse={evaluator} />
      <TaskOutputPanel key={selectedTask?.id ?? "no-task"} runId={selectedRun} task={selectedTask} />
      <SamplerAggregatorPanel run={currentRun} panelResponse={sampler} />
      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
    </>
  );
};

const RunsWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected }) => (
  <RunScopedWorkspace
    runs={runs}
    selectedRun={selectedRun}
    setSelectedRun={setSelectedRun}
    isConnected={isConnected}
    noRunsMessage="Create a run to start monitoring task output and engine configuration."
    noSelectionMessage="Pick a run to view task-scoped output and run configuration."
  >
    <RunModeContent runs={runs} selectedRun={selectedRun} />
  </RunScopedWorkspace>
);

function AppContent() {
  const { runs, isConnected } = useRuns();
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
  const [mode, setMode] = useState("runs");
  const [selectedRun, setSelectedRun] = useState(null);
  const [selectedLogRun, setSelectedLogRun] = useState(null);
  const runList = asArray(runs);

  useEffect(() => {
    if (runList.length === 0) {
      setSelectedRun(null);
      setSelectedLogRun(null);
      return;
    }

    if (!selectedRun || !runList.some((run) => run.run_id === selectedRun)) {
      setSelectedRun(runList[0].run_id);
    }

    if (!selectedLogRun || !runList.some((run) => run.run_id === selectedLogRun)) {
      setSelectedLogRun(runList[0].run_id);
    }
  }, [runList, selectedRun, selectedLogRun]);

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <DashboardHeader />
      <LoginDialog />

      <Tabs value={mode} onChange={(_, next) => setMode(next)} sx={{ mb: 3 }}>
        <Tab value="runs" label="Runs" />
        <Tab value="workers" label="Nodes" />
        <Tab value="performance" label="Performance" />
        <Tab value="logs" label="Logs" />
      </Tabs>

      {mode === "runs" ? (
        <RunsWorkspace
          runs={runList}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
        />
      ) : mode === "workers" ? (
        <WorkersWorkspace
          workers={workersData.workers}
          runs={runList}
          isConnected={workersData.isConnected}
          lastUpdate={workersData.lastUpdate}
          error={workersData.error}
        />
      ) : mode === "performance" ? (
        <PerformanceWorkspace
          runs={runList}
          workers={workersData.workers}
          selectedRun={selectedRun}
          setSelectedRun={setSelectedRun}
          isConnected={isConnected}
        />
      ) : (
        <LogsWorkspace
          runs={runList}
          workers={workersData.workers}
          selectedRun={selectedLogRun}
          setSelectedRun={setSelectedLogRun}
          isConnected={isConnected}
        />
      )}
    </Container>
  );
}

function App() {
  return (
    <AuthProvider>
      <AppContent />
    </AuthProvider>
  );
}

export default App;
