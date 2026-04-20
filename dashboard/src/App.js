import { Alert, Box, Button, Chip, Container, Snackbar, Stack, Tab, Tabs, TextField, Typography } from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import gammaboardLogo from "./assets/gammalooplogo.svg";
import { AuthProvider, useAuth } from "./auth/AuthProvider";
import EvaluatorPanel from "./components/EvaluatorPanel";
import LogsWorkspace from "./components/LogsWorkspace";
import PerformanceWorkspace from "./components/PerformanceWorkspace";
import SamplerAggregatorPanel from "./components/SamplerAggregatorPanel";
import SelectedTaskTomlPanel from "./components/SelectedTaskTomlPanel";
import TaskOutputPanel from "./components/TaskOutputPanel";
import TaskQueuePanel from "./components/TaskQueuePanel";
import WorkersWorkspace from "./components/WorkersWorkspace";
import LoginDialog from "./components/auth/LoginDialog";
import RunScopedWorkspace from "./components/common/RunScopedWorkspace";
import CloneRunDialog from "./components/runs/CloneRunDialog";
import QueueTuningPanel from "./components/runs/QueueTuningPanel";
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
  deleteTemplateFile,
  fetchNodes,
  fetchTemplateFile,
  fetchTemplateList,
  fetchRunReproToml,
  pauseRun,
  updateRunTaskQueueTuning,
  saveTemplateFile,
  unassignNode,
} from "./services/api";
import { copyToClipboard } from "./utils/clipboard";
import { asArray } from "./utils/collections";
import { asTaskList, getCurrentTask } from "./utils/tasks";

const DEFAULT_CREATE_RUN_TOML = `name = "new-run"`;

const DEFAULT_ADD_TASKS_TOML = `[[task_queue]]
kind = "sample"
accumulator = { config = "scalar" }

stop_condition = { max_samples = 10000 }
`;
const EVALUATOR_COUNT_STORAGE_KEY = "runs.evaluator_count";

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
  const [queueTuningBusy, setQueueTuningBusy] = useState(false);
  const [cloneRunError, setCloneRunError] = useState(null);
  const [addTasksError, setAddTasksError] = useState(null);
  const [taskTemplates, setTaskTemplates] = useState([]);
  const [evaluatorCount, setEvaluatorCount] = useState(() => {
    if (typeof window === "undefined") return "";
    const stored = window.localStorage.getItem(EVALUATOR_COUNT_STORAGE_KEY);
    return stored && /^\d+$/.test(stored) ? stored : "";
  });
  const [autoUnassigning, setAutoUnassigning] = useState(false);
  const { authenticated } = useAuth();

  const reloadTaskTemplates = useCallback(async () => {
    try {
      const items = await fetchTemplateList("tasks");
      setTaskTemplates(items);
    } catch (err) {
      console.error("Failed to fetch task templates:", err);
    }
  }, []);

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
    if (typeof window === "undefined") return;
    if (evaluatorCount && /^\d+$/.test(evaluatorCount)) {
      window.localStorage.setItem(EVALUATOR_COUNT_STORAGE_KEY, evaluatorCount);
    } else {
      window.localStorage.removeItem(EVALUATOR_COUNT_STORAGE_KEY);
    }
  }, [evaluatorCount]);

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

  const parseEvaluatorTarget = (value) => {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed) || parsed < 1) return null;
    return Math.floor(parsed);
  };

  return (
    <>
      {authenticated ? (
        <Box sx={{ mb: 2, display: "flex", justifyContent: "flex-end" }}>
          <Stack direction={{ xs: "column", md: "row" }} spacing={1.5}>
            <TextField
              size="small"
              value={evaluatorCount}
              onChange={(event) => setEvaluatorCount(event.target.value.replace(/[^\d]/g, ""))}
              placeholder="all"
              inputProps={{ "aria-label": "Evaluator node count" }}
              sx={{ minWidth: 160 }}
            />
            <Button
              variant="contained"
              disabled={!selectedRun || pausing || autoAssigning || autoUnassigning}
              onClick={async () => {
                  setAutoAssigning(true);
                  try {
                  const limit = parseEvaluatorTarget(evaluatorCount);
                  if (evaluatorCount.trim() && limit == null) {
                    setSnackbar({ message: "N must be at least 1.", severity: "error" });
                    return;
                  }
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
              Assign
            </Button>
            <Button
              variant="contained"
              color="warning"
              disabled={!selectedRun || pausing || autoAssigning || autoUnassigning}
              onClick={async () => {
                setAutoUnassigning(true);
                try {
                  const requested = parseEvaluatorTarget(evaluatorCount);
                  if (evaluatorCount.trim() && requested == null) {
                    setSnackbar({ message: "N must be at least 1.", severity: "error" });
                    return;
                  }
                  const nodes = await fetchNodes(selectedRun);
                  const assignedEvaluators = asArray(nodes).filter(
                    (worker) =>
                      worker?.node_name &&
                      worker?.desired_run_id === selectedRun &&
                      worker?.desired_role === "evaluator",
                  );
                  const target = requested == null ? assignedEvaluators.length : Math.max(0, requested);
                  const evaluators = assignedEvaluators.slice(0, target);
                  if (evaluators.length === 0) {
                    setSnackbar({ message: "No evaluator nodes assigned to this run.", severity: "info" });
                    return;
                  }
                  await Promise.all(evaluators.map((worker) => unassignNode(worker.node_name)));
                  setSnackbar({
                    message: `Requested unassign for ${evaluators.length} evaluator node${evaluators.length === 1 ? "" : "s"}.`,
                    severity: "success",
                  });
                } catch (err) {
                  setSnackbar({ message: err?.message || "Failed to unassign evaluator nodes.", severity: "error" });
                } finally {
                  setAutoUnassigning(false);
                }
              }}
            >
              Unassign
            </Button>
            <Button
              variant="contained"
              color="warning"
              disabled={!selectedRun || pausing || autoAssigning || autoUnassigning || deletingRun}
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
              disabled={
                !selectedRun || pausing || autoAssigning || autoUnassigning || deletingRun || cloneRunBusy || addTasksBusy
              }
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
                disabled={
                  !selectedRun ||
                  cloneRunBusy ||
                  addTasksBusy ||
                  deletingRun ||
                  (!selectedTask?.latest_stage_snapshot_id && !currentRun?.root_stage_snapshot_id)
                }
                onClick={() => {
                  setCloneRunError(null);
                  setCloneRunOpen(true);
                }}
              >
                Clone Run
              </Button>
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
      <SelectedTaskTomlPanel task={selectedTask} />
      <TaskOutputPanel
        key={`progress-${selectedTask?.id ?? "no-task"}`}
        runId={selectedRun}
        task={selectedTask}
        includePanelIds={["sample_progress"]}
        title="Task Progress"
      />
      <TaskOutputPanel
        key={selectedTask?.id ?? "no-task"}
        runId={selectedRun}
        task={selectedTask}
        excludePanelIds={["sample_progress"]}
      />
      <QueueTuningPanel
        run={currentRun}
        runId={selectedRun}
        task={selectedTask}
        authenticated={authenticated}
        busy={queueTuningBusy}
        onSave={async (payload) => {
          if (!selectedRun || !selectedTask?.id) return;
          setQueueTuningBusy(true);
          try {
            await updateRunTaskQueueTuning(selectedRun, selectedTask.id, payload);
            setSnackbar({ message: "Queue tuning updated.", severity: "success" });
          } catch (err) {
            setSnackbar({ message: err?.message || "Failed to update queue tuning.", severity: "error" });
          } finally {
            setQueueTuningBusy(false);
          }
        }}
        onClear={async () => {
          if (!selectedRun || !selectedTask?.id) return;
          setQueueTuningBusy(true);
          try {
            await updateRunTaskQueueTuning(selectedRun, selectedTask.id, null);
            setSnackbar({ message: "Queue tuning override cleared.", severity: "success" });
          } catch (err) {
            setSnackbar({ message: err?.message || "Failed to clear queue tuning override.", severity: "error" });
          } finally {
            setQueueTuningBusy(false);
          }
        }}
      />
      <EvaluatorPanel run={currentRun} panelResponse={evaluator} />
      <SamplerAggregatorPanel run={currentRun} panelResponse={sampler} />
      <CloneRunDialog
        open={cloneRunOpen}
        initialName={cloneInitialName}
        busy={cloneRunBusy}
        error={cloneRunError}
        onClose={closeCloneRun}
        onSubmit={async ({ newName }) => {
          const fromSnapshotId = selectedTask?.latest_stage_snapshot_id ?? currentRun?.root_stage_snapshot_id ?? null;
          if (!selectedRun || !fromSnapshotId) {
            setCloneRunError("No source snapshot is available for cloning.");
            return;
          }
          setCloneRunBusy(true);
          setCloneRunError(null);
          try {
            const response = await cloneRun({ sourceRunId: selectedRun, fromSnapshotId, newName });
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
        helperText='Submit one or more [[task_queue]] entries using sampler_aggregator / accumulator sources: omitted = latest, or { from_name = "..." }, or { config = ... }.'
        templates={taskTemplates}
        loadTemplate={async (name) => {
          const response = await fetchTemplateFile("tasks", name);
          return response?.toml || "";
        }}
        onSaveTemplate={async (name, toml) => {
          const response = await saveTemplateFile("tasks", { name, toml });
          await reloadTaskTemplates();
          setSnackbar({ message: `Saved task template "${response?.name || name}".`, severity: "success" });
          return response;
        }}
        onDeleteTemplate={async (name) => {
          await deleteTemplateFile("tasks", name);
          await reloadTaskTemplates();
          setSnackbar({ message: `Deleted task template "${name}".`, severity: "success" });
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
  const [copyRunBusy, setCopyRunBusy] = useState(false);
  const [createRunError, setCreateRunError] = useState(null);
  const [snackbar, setSnackbar] = useState(null);
  const [runTemplates, setRunTemplates] = useState([]);

  const reloadRunTemplates = useCallback(async () => {
    try {
      const items = await fetchTemplateList("runs");
      setRunTemplates(items);
    } catch (err) {
      console.error("Failed to fetch run templates:", err);
    }
  }, []);

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
          <Box sx={{ mb: 2, display: "flex", justifyContent: "flex-end" }}>
            <Stack direction="row" spacing={1}>
              <Button
                variant="outlined"
                disabled={!selectedRun || copyRunBusy}
                onClick={async () => {
                  if (!selectedRun) return;
                  setCopyRunBusy(true);
                  try {
                    const response = await fetchRunReproToml(selectedRun);
                    await copyToClipboard(response?.toml || "");
                    setSnackbar({ message: "Run reproduction TOML copied.", severity: "success" });
                  } catch (err) {
                    setSnackbar({ message: err?.message || "Failed to copy run TOML.", severity: "error" });
                  } finally {
                    setCopyRunBusy(false);
                  }
                }}
              >
                Copy Run TOML
              </Button>
              {authenticated ? (
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
              ) : null}
            </Stack>
          </Box>
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
        helperText="Enter a run config. The backend merges this with configs/runs/default.toml."
        templates={runTemplates}
        loadTemplate={async (name) => {
          const response = await fetchTemplateFile("runs", name);
          return response?.toml || "";
        }}
        onSaveTemplate={async (name, toml) => {
          const response = await saveTemplateFile("runs", { name, toml });
          await reloadRunTemplates();
          setSnackbar({ message: `Saved run template "${response?.name || name}".`, severity: "success" });
          return response;
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
