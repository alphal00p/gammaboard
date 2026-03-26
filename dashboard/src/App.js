import { Alert, Box, Button, Chip, Container, Snackbar, Stack, Tab, Tabs, TextField, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
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
import CloneRunDialog from "./components/runs/CloneRunDialog";
import TomlActionDialog from "./components/runs/TomlActionDialog";
import { useRunConfigPanels } from "./hooks/useRunConfigPanels";
import { useRuns } from "./hooks/useRuns";
import { useRunTasks } from "./hooks/useRunTasks";
import { useWorkersData } from "./hooks/useWorkersData";
import {
  addRunTasks,
  autoAssignRun,
  cloneRun,
  createRun,
  deleteRun,
  deleteRunTask,
  fetchTemplateFile,
  fetchTemplateList,
  pauseRun,
} from "./services/api";
import { asArray } from "./utils/collections";
import { asTaskList, getCurrentTask } from "./utils/tasks";

const DEFAULT_CREATE_RUN_TOML = `name = "new-run"`;

const DEFAULT_ADD_TASKS_TOML = `[[task_queue]]
kind = "sample"
nr_samples = 10000
observable = { config = "scalar" }`;

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

const RunModeContent = ({ runs, selectedRun, onRunCreated, onRunDeleted }) => {
  const currentRun = runs.find((entry) => entry.run_id === selectedRun);
  const { tasks } = useRunTasks(selectedRun, 2000);
  const { evaluator, sampler } = useRunConfigPanels({ runId: selectedRun, pollMs: 5000 });
  const [cloneSourceRunId, setCloneSourceRunId] = useState(selectedRun);
  const { tasks: cloneSourceTasks } = useRunTasks(cloneSourceRunId, 2000);
  const [selectedTaskId, setSelectedTaskId] = useState(null);
  const [snackbar, setSnackbar] = useState(null);
  const [pausing, setPausing] = useState(false);
  const [deletingRun, setDeletingRun] = useState(false);
  const [deletingTask, setDeletingTask] = useState(false);
  const [autoAssigning, setAutoAssigning] = useState(false);
  const [cloneRunOpen, setCloneRunOpen] = useState(false);
  const [addTasksOpen, setAddTasksOpen] = useState(false);
  const [cloneRunBusy, setCloneRunBusy] = useState(false);
  const [addTasksBusy, setAddTasksBusy] = useState(false);
  const [cloneRunError, setCloneRunError] = useState(null);
  const [addTasksError, setAddTasksError] = useState(null);
  const [taskTemplates, setTaskTemplates] = useState([]);
  const [maxEvaluators, setMaxEvaluators] = useState("");
  const { authenticated } = useAuth();

  useEffect(() => {
    let cancelled = false;
    fetchTemplateList("tasks")
      .then((items) => {
        if (!cancelled) setTaskTemplates(items);
      })
      .catch((err) => {
        console.error("Failed to fetch task templates:", err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (selectedRun != null) {
      setCloneSourceRunId(selectedRun);
    }
  }, [selectedRun]);

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

  const taskList = asTaskList(tasks);
  const selectedTask = taskList.find((task) => task.id === selectedTaskId) ?? getCurrentTask(taskList) ?? null;
  const cloneInitialName = useMemo(() => {
    if (!currentRun?.run_name) return "cloned-run";
    return `${currentRun.run_name}-clone`;
  }, [currentRun]);

  if (!currentRun) {
    return (
      <Alert severity="warning" sx={{ mb: 3 }}>
        Selected run not found in current run list.
      </Alert>
    );
  }

  const closeCloneRun = () => {
    if (cloneRunBusy) return;
    setCloneRunError(null);
    setCloneRunOpen(false);
  };

  const closeAddTasks = () => {
    if (addTasksBusy) return;
    setAddTasksError(null);
    setAddTasksOpen(false);
  };

  return (
    <>
      {authenticated ? (
        <Box sx={{ mb: 2, display: "flex", justifyContent: "flex-end" }}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
            <Button
              variant="outlined"
              disabled={!selectedRun || cloneRunBusy || addTasksBusy || pausing || autoAssigning}
              onClick={() => {
                setCloneRunError(null);
                setCloneRunOpen(true);
              }}
            >
              Clone Run
            </Button>
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
              disabled={!selectedRun || pausing || autoAssigning || deletingRun}
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
            <Button
              variant="outlined"
              color="error"
              disabled={!selectedRun || pausing || autoAssigning || deletingRun || cloneRunBusy || addTasksBusy}
              onClick={async () => {
                if (!window.confirm("Delete this run? This cannot be undone.")) return;
                setDeletingRun(true);
                try {
                  await deleteRun(selectedRun);
                  onRunDeleted?.(selectedRun);
                  setSnackbar({ message: "Run deleted.", severity: "success" });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to delete run.", severity: "error" });
                } finally {
                  setDeletingRun(false);
                }
              }}
            >
              Delete Run
            </Button>
          </Stack>
        </Box>
      ) : null}
      <RunInfo runId={selectedRun} />
      <TaskQueuePanel
        tasks={taskList}
        selectedTaskId={selectedTask?.id ?? null}
        onSelectTask={setSelectedTaskId}
        actions={
          authenticated ? (
            <Stack direction="row" spacing={1}>
              <Button
                size="small"
                variant="outlined"
                disabled={!selectedRun || addTasksBusy || cloneRunBusy || deletingRun}
                onClick={() => {
                  setAddTasksError(null);
                  setAddTasksOpen(true);
                }}
              >
                Add Task
              </Button>
              <Button
                size="small"
                variant="outlined"
                color="error"
                disabled={!selectedRun || deletingTask || deletingRun || selectedTask?.state !== "pending"}
                onClick={async () => {
                  if (!selectedTask?.id) return;
                  if (!window.confirm(`Delete pending task "${selectedTask.name}"?`)) return;
                  setDeletingTask(true);
                  try {
                    await deleteRunTask(selectedRun, selectedTask.id);
                    setSnackbar({ message: "Pending task deleted.", severity: "success" });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to delete pending task.", severity: "error" });
                  } finally {
                    setDeletingTask(false);
                  }
                }}
              >
                Delete Task
              </Button>
            </Stack>
          ) : null
        }
      />
      <EvaluatorPanel run={currentRun} panelResponse={evaluator} />
      <TaskOutputPanel key={selectedTask?.id ?? "no-task"} runId={selectedRun} task={selectedTask} />
      <SamplerAggregatorPanel run={currentRun} panelResponse={sampler} />
      <CloneRunDialog
        open={cloneRunOpen}
        runs={runs}
        sourceRunId={cloneSourceRunId}
        setSourceRunId={setCloneSourceRunId}
        sourceTasks={cloneSourceTasks}
        initialName={cloneInitialName}
        busy={cloneRunBusy}
        error={cloneRunError}
        onClose={closeCloneRun}
        onSubmit={async ({ sourceRunId, fromSnapshotId, newName }) => {
          setCloneRunBusy(true);
          setCloneRunError(null);
          try {
            const response = await cloneRun({ sourceRunId, fromSnapshotId, newName });
            setCloneRunOpen(false);
            setSnackbar({
              message: `Cloned run ${response?.run_name || "run"} (#${response?.run_id ?? "?"}).`,
              severity: "success",
            });
            if (Number.isFinite(Number(response?.run_id))) {
              onRunCreated(Number(response.run_id));
            }
          } catch (err) {
            setCloneRunError(err?.message || "Failed to clone run.");
          } finally {
            setCloneRunBusy(false);
          }
        }}
      />
      <TomlActionDialog
        open={addTasksOpen}
        title="Add Tasks"
        label="Task Queue TOML"
        submitLabel="Add Tasks"
        initialValue={DEFAULT_ADD_TASKS_TOML}
        helperText='Submit one or more [[task_queue]] entries using sampler_aggregator / observable sources: omitted = latest, or { from_name = "..." }, or { config = ... }.'
        templates={taskTemplates}
        loadTemplate={async (name) => {
          const response = await fetchTemplateFile("tasks", name);
          return response?.toml || "";
        }}
        busy={addTasksBusy}
        error={addTasksError}
        onClose={closeAddTasks}
        onSubmit={async (toml) => {
          setAddTasksBusy(true);
          setAddTasksError(null);
          try {
            const inserted = await addRunTasks(selectedRun, toml);
            setAddTasksOpen(false);
            setSnackbar({
              message: `Added ${Array.isArray(inserted) ? inserted.length : 0} task(s).`,
              severity: "success",
            });
          } catch (err) {
            setAddTasksError(err?.message || "Failed to add tasks.");
          } finally {
            setAddTasksBusy(false);
          }
        }}
      />
      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
    </>
  );
};

const RunsWorkspace = ({ runs, selectedRun, setSelectedRun, isConnected, onRunCreated }) => {
  const { authenticated } = useAuth();
  const [createRunOpen, setCreateRunOpen] = useState(false);
  const [createRunBusy, setCreateRunBusy] = useState(false);
  const [createRunError, setCreateRunError] = useState(null);
  const [snackbar, setSnackbar] = useState(null);
  const [runTemplates, setRunTemplates] = useState([]);

  useEffect(() => {
    let cancelled = false;
    fetchTemplateList("runs")
      .then((items) => {
        if (!cancelled) setRunTemplates(items);
      })
      .catch((err) => {
        console.error("Failed to fetch run templates:", err);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <RunScopedWorkspace
        runs={runs}
        selectedRun={selectedRun}
        setSelectedRun={setSelectedRun}
        isConnected={isConnected}
        noRunsMessage="Create a run to start monitoring task output and engine configuration."
        noSelectionMessage="Pick a run to view task-scoped output and run configuration."
        headerActions={
          authenticated ? (
            <Box sx={{ mb: 2, display: "flex", justifyContent: "flex-end" }}>
              <Button
                variant="outlined"
                disabled={createRunBusy}
                onClick={() => {
                  setCreateRunError(null);
                  setCreateRunOpen(true);
                }}
              >
                New Run
              </Button>
            </Box>
          ) : null
        }
      >
        <RunModeContent
          runs={runs}
          selectedRun={selectedRun}
          onRunCreated={onRunCreated}
          onRunDeleted={(runId) => {
            if (selectedRun === runId) {
              setSelectedRun(null);
            }
          }}
        />
      </RunScopedWorkspace>
      <TomlActionDialog
        open={createRunOpen}
        title="Create Run"
        label="Run TOML"
        submitLabel="Create Run"
        initialValue={DEFAULT_CREATE_RUN_TOML}
        helperText="Enter a run config. The backend merges this with configs/default.toml."
        templates={runTemplates}
        loadTemplate={async (name) => {
          const response = await fetchTemplateFile("runs", name);
          return response?.toml || "";
        }}
        busy={createRunBusy}
        error={createRunError}
        onClose={() => {
          if (createRunBusy) return;
          setCreateRunError(null);
          setCreateRunOpen(false);
        }}
        onSubmit={async (toml) => {
          setCreateRunBusy(true);
          setCreateRunError(null);
          try {
            const response = await createRun(toml);
            setCreateRunOpen(false);
            setSnackbar({
              message: `Created run ${response?.run_name || "run"} (#${response?.run_id ?? "?"}).`,
              severity: "success",
            });
            if (Number.isFinite(Number(response?.run_id))) {
              onRunCreated(Number(response.run_id));
            }
          } catch (err) {
            setCreateRunError(err?.message || "Failed to create run.");
          } finally {
            setCreateRunBusy(false);
          }
        }}
      />
      <Snackbar
        open={Boolean(snackbar)}
        autoHideDuration={4000}
        onClose={() => setSnackbar(null)}
        message={snackbar?.message || ""}
      />
    </>
  );
};

function AppContent() {
  const { runs, isConnected } = useRuns();
  const workersData = useWorkersData({ runId: null, pollMs: 3000 });
  const [mode, setMode] = useState("runs");
  const [selectedRun, setSelectedRun] = useState(null);
  const [selectedLogRun, setSelectedLogRun] = useState(null);
  const [pendingRunSelection, setPendingRunSelection] = useState(null);
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

  useEffect(() => {
    if (pendingRunSelection == null) return;
    if (!runList.some((run) => run.run_id === pendingRunSelection)) return;
    setSelectedRun(pendingRunSelection);
    setMode("runs");
    setPendingRunSelection(null);
  }, [pendingRunSelection, runList]);

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
          onRunCreated={setPendingRunSelection}
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
