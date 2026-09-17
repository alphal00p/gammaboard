import { Alert, Box, Button, Snackbar, Stack, TextField } from "@mui/material";
import { lazy, useEffect, useMemo, useState } from "react";
import { useAuth } from "../../auth/AuthProvider";
import { useRunTasks } from "../../hooks/useRunTasks";
import {
  addRunTasks,
  autoAssignRun,
  cloneRun,
  createRun,
  deleteRun,
  deleteRunTask,
  fetchNodes,
  fetchRunReproToml,
  pauseRun,
  unassignNode,
} from "../../services/api";
import { copyToClipboard } from "../../utils/clipboard";
import { asArray } from "../../utils/collections";
import { getCurrentTask } from "../../utils/tasks";
import RunScopedWorkspace from "../common/RunScopedWorkspace";

const CloneRunDialog = lazy(() => import("./CloneRunDialog"));
const RunInfo = lazy(() => import("../RunInfo"));
const TaskOutputPanel = lazy(() => import("../TaskOutputPanel"));
const TaskQueuePanel = lazy(() => import("../TaskQueuePanel"));
const TomlActionDialog = lazy(() => import("./TomlActionDialog"));

const EVALUATOR_COUNT_STORAGE_KEY = "runs.evaluator_count";
const CREATE_RUN_TEMPLATE_SELECTION_STORAGE_KEY = "dialogs.create_run.selected_template";
const ADD_TASKS_TEMPLATE_SELECTION_STORAGE_KEY = "dialogs.add_tasks.selected_template";

const RunModeContent = ({ runs, selectedRun, onRunCreated, onRunDeleted, onSelectRun }) => {
  const currentRun = runs.find((entry) => entry.run_id === selectedRun);
  const isIntegration = !currentRun?.kind || currentRun.kind === "integration";
  const { tasks } = useRunTasks(selectedRun, 2000);
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
  const [evaluatorCount, setEvaluatorCount] = useState(() => {
    if (typeof window === "undefined") return "";
    const stored = window.localStorage.getItem(EVALUATOR_COUNT_STORAGE_KEY);
    return stored && /^\d+$/.test(stored) ? stored : "";
  });
  const [autoUnassigning, setAutoUnassigning] = useState(false);
  const { authenticated } = useAuth();

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (evaluatorCount && /^\d+$/.test(evaluatorCount)) {
      window.localStorage.setItem(EVALUATOR_COUNT_STORAGE_KEY, evaluatorCount);
    } else {
      window.localStorage.removeItem(EVALUATOR_COUNT_STORAGE_KEY);
    }
  }, [evaluatorCount]);

  useEffect(() => {
    const taskList = asArray(tasks);
    if (taskList.length === 0) {
      setSelectedTaskId(null);
      return;
    }
    if (selectedTaskId != null && taskList.some((task) => task.id === selectedTaskId)) {
      return;
    }
    setSelectedTaskId(getCurrentTask(taskList)?.id ?? taskList[0].id ?? null);
  }, [selectedTaskId, tasks]);

  const taskList = asArray(tasks);
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
                !selectedRun ||
                pausing ||
                autoAssigning ||
                autoUnassigning ||
                deletingRun ||
                cloneRunBusy ||
                addTasksBusy
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
      {isIntegration && <TaskQueuePanel
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
      />}
      {(selectedTask || isIntegration) && <TaskOutputPanel
        title={isIntegration ? "Selected Task Output" : "Run Output"}
        key={selectedTask?.id ?? "no-task"}
        runId={selectedRun}
        task={selectedTask}
        onSelectRun={onSelectRun}
      />}
      <RunInfo runId={selectedRun} />
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
        initialValue=""
        helperText='Submit one or more [[task_queue]] entries using sampler_aggregator / accumulator sources: omitted = latest, or { from_name = "..." }, or { config = ... }.'
        templateKind="tasks"
        allowTemplateDelete
        onTemplateSaved={(response, name) => {
          setSnackbar({ message: `Saved task template "${response?.name || name}".`, severity: "success" });
        }}
        templateSelectionStorageKey={ADD_TASKS_TEMPLATE_SELECTION_STORAGE_KEY}
        onTemplateDeleted={(name) => {
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

const RunsWorkspace = ({
  runs,
  selectedRun,
  setSelectedRun,
  showChildRuns,
  setShowChildRuns,
  isConnected,
  serverName,
  onRunCreated,
  onSelectRun,
  hasMoreRuns,
  loadMoreRuns,
  isLoadingMoreRuns,
}) => {
  const { authenticated } = useAuth();
  const [createRunOpen, setCreateRunOpen] = useState(false);
  const [createRunBusy, setCreateRunBusy] = useState(false);
  const [copyRunBusy, setCopyRunBusy] = useState(false);
  const [createRunError, setCreateRunError] = useState(null);
  const [snackbar, setSnackbar] = useState(null);

  return (
    <>
      <RunScopedWorkspace
        runs={runs}
        selectedRun={selectedRun}
        setSelectedRun={setSelectedRun}
        showChildRuns={showChildRuns}
        setShowChildRuns={setShowChildRuns}
        hasMoreRuns={hasMoreRuns}
        loadMoreRuns={loadMoreRuns}
        isLoadingMoreRuns={isLoadingMoreRuns}
        isConnected={isConnected}
        serverName={serverName}
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
          onSelectRun={onSelectRun}
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
        initialValue=""
        helperText="Enter a run config. The backend merges this with the built-in default run template."
        templateKind="runs"
        onTemplateSaved={(response, name) => {
          setSnackbar({ message: `Saved run template "${response?.name || name}".`, severity: "success" });
        }}
        templateSelectionStorageKey={CREATE_RUN_TEMPLATE_SELECTION_STORAGE_KEY}
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


export default RunsWorkspace;

