import LoadingStatus from "../common/LoadingStatus";
import { Alert, Box, Button, IconButton, Snackbar, TextField, Tooltip } from "@mui/material";
import DeleteOutlineIcon from "@mui/icons-material/DeleteOutline";
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import EditOutlinedIcon from "@mui/icons-material/EditOutlined";
import { lazy, useEffect, useRef, useState } from "react";
import { useAuth } from "../../auth/AuthProvider";
import { useTaskOutput } from "../../hooks/useTaskOutput";
import TaskSummary from "../TaskSummary";
import { useRunTasks } from "../../hooks/useRunTasks";
import { addRunTasks, autoAssignRun, createRun, deleteRun, deleteRunTask, editRunTask,
  fetchNodes, fetchRunDefinition, fetchTaskDefinition, pauseRun, unassignNode } from "../../services/api";
import { asArray } from "../../utils/collections";
import { getCurrentTask } from "../../utils/tasks";
import { workerAssignmentReason } from "../../utils/workerActions";
import RunScopedWorkspace from "../common/RunScopedWorkspace";
import WorkspaceTabs, { TabLoadingScope, WorkspacePanel, useWorkspaceActive } from "../common/WorkspaceTabs";
import { ControlBar } from "../common/PanelChrome";
import StableTaskOutput from "../common/StableTaskOutput";

const PerformanceWorkspace = lazy(() => import("../PerformanceWorkspace"));
const RunInfo = lazy(() => import("../RunInfo"));
const TaskOutputPanel = lazy(() => import("../TaskOutputPanel"));
const TaskQueuePanel = lazy(() => import("../TaskQueuePanel"));
const TomlActionDialog = lazy(() => import("./TomlActionDialog"));
const EXTERNAL_PATH_WARNING = "External files are not copied or verified. Review writable output paths so independent runs do not overwrite each other's files.";

function DefinitionButtons({ openLabel, onOpen, editable, disabled, onDelete, deleteLabel, deleteReason }) {
  return <Box sx={{ display: "inline-flex" }}>
    <Tooltip title={openLabel}><span>
      <IconButton size="small" aria-label={openLabel} disabled={disabled} onClick={onOpen}>
        {editable ? <EditOutlinedIcon fontSize="small" /> : <DescriptionOutlinedIcon fontSize="small" />}
      </IconButton>
    </span></Tooltip>
    {onDelete && <Tooltip title={deleteReason || deleteLabel}><span>
      <IconButton size="small" color="error" aria-label={deleteLabel}
        disabled={disabled || Boolean(deleteReason)} onClick={onDelete}>
        <DeleteOutlineIcon fontSize="small" />
      </IconButton>
    </span></Tooltip>}
  </Box>;
}

function WorkerPoolControls({ run, workers, disabled, notify }) {
  const [busy, setBusy] = useState(false);
  const [operation, setOperation] = useState("");
  const [count, setCount] = useState(() => window.localStorage.getItem("runs.evaluator_count") || "");
  const pool = asArray(workers).filter((worker) => worker.pool_run_id === run.run_id);
  const assignReason = workerAssignmentReason(run);
  const removeReason = pool.some((worker) => worker.pool_role === "evaluator") ? "" : "No evaluators in this run's pool.";
  const pauseReason = ["paused", "pausing"].includes(run.lifecycle_state) ? "This run is already paused or pausing."
    : pool.length ? "" : "No workers in this run's pool.";
  const act = async (action) => {
    setOperation(action === "pause" ? "Requesting pause…" : action === "remove" ? "Removing evaluators…" : "Assigning workers…");
    setBusy(true);
    try {
      const target = count.trim() ? Number(count) : null;
      if (action !== "pause" && target != null && (!Number.isSafeInteger(target) || target < 1)) throw new Error("N must be at least 1.");
      if (count) window.localStorage.setItem("runs.evaluator_count", count);
      else window.localStorage.removeItem("runs.evaluator_count");
      if (action === "assign") {
        const result = await autoAssignRun(run.run_id, { maxEvaluators: target });
        notify(`Assigned ${(result.assigned_evaluators?.length || 0) + (result.assigned_samplers?.length || 0)} node(s); resumed ${result.resumed_nodes || 0} pool member(s).`);
      } else if (action === "remove") {
        const nodes = await fetchNodes(run.run_id);
        const evaluators = asArray(nodes).filter((node) => node.pool_run_id === run.run_id && node.pool_role === "evaluator");
        const selected = target == null ? evaluators : evaluators.slice(0, target);
        await Promise.all(selected.map((node) => unassignNode(node.node_name)));
        notify(`Requested unassign for ${selected.length} evaluator node(s).`);
      } else {
        await pauseRun(run.run_id);
        notify("Pause requested.");
      }
    } catch (error) { notify(error.message, "error"); }
    finally { setBusy(false); }
  };
  return <ControlBar>
    <LoadingStatus loading={busy} label={operation} />
    <TextField size="small" label="Evaluators" value={count} placeholder="all" InputLabelProps={{ shrink: true }} disabled={disabled || busy || Boolean(assignReason && removeReason)} inputProps={{ "aria-label": "Evaluator node count" }} sx={{ width: 104 }}
      onChange={(event) => setCount(event.target.value.replace(/[^\d]/g, ""))} />
    {[["assign", "Assign / resume", assignReason], ["remove", "Remove evaluators", removeReason], ["pause", "Pause run", pauseReason]].map(([action, label, reason]) =>
      <Tooltip key={action} title={reason}><span>
        <Button variant={action === "assign" ? "contained" : "outlined"} color={action === "assign" ? "primary" : "warning"}
          disabled={disabled || busy || Boolean(reason)} onClick={() => act(action)}>{label}</Button>
      </span></Tooltip>)}
  </ControlBar>;
}

function RunContent({ run, runs, workers, isConnected, disabled, notify, onSelectRun, navigation, onNavigate, runActions }) {
  const [localView, setLocalView] = useState("tasks");
  const [localTaskTab, setLocalTaskTab] = useState("result");
  const taskTab = navigation?.taskTab ?? localTaskTab;
  const setTaskTab = (taskTab, options) => onNavigate ? onNavigate({ taskTab }, options) : setLocalTaskTab(taskTab);
  const view = navigation?.runTab ?? localView;
  const setView = (runTab) => onNavigate ? onNavigate({ runTab }) : setLocalView(runTab);
  const { authenticated } = useAuth();
  const { tasks, isLoading: tasksLoading, error: tasksError } = useRunTasks(run.run_id, 2000);
  const [selectedId, setSelectedId] = useState(null);
  const loading = useRef(null);
  useEffect(() => () => loading.current?.abort(), []);
  const [editor, setEditor] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const isIntegration = !run.kind || run.kind === "integration";
  const editable = authenticated && isIntegration && run.parent_run_id == null;
  const taskList = asArray(tasks);
  const requestedTask = navigation?.taskId ?? selectedId;
  const selectedTask = taskList.find((task) => String(task.id) === String(requestedTask)) ?? getCurrentTask(taskList);
  const taskOutput = useTaskOutput({ runId: run.run_id, taskId: selectedTask?.id ?? null,
    section: view === "summary" ? "summary" : taskTab,
    refreshKey: selectedTask?.state, paused: !["tasks", "summary"].includes(view) });
  const selectTask = (taskId) => { setSelectedId(taskId); onNavigate?.({ taskId: String(taskId) }); };
  useEffect(() => {
    if (!tasksLoading && selectedTask && String(requestedTask) !== String(selectedTask.id)) {
      setSelectedId(selectedTask.id);
      onNavigate?.({ taskId: String(selectedTask.id) }, { replace: true });
    }
  }, [tasksLoading, selectedTask, requestedTask, onNavigate]);
  const canSave = editable && taskList.find((task) => task.id === editor?.taskId)?.state === "pending";
  const openTask = async (task) => {
    setBusy(true);
    setError(null);
    loading.current?.abort();
    const request = new AbortController();
    loading.current = request;
    try {
      const { toml } = await fetchTaskDefinition(run.run_id, task.id, false, request.signal);
      if (request.signal.aborted) return;
      setEditor({ title: `Task definition: ${task.name}`, name: task.name, value: toml, taskId: task.id });
    } catch (error) { if (!request.signal.aborted) notify(error.message, "error"); }
    finally { if (!request.signal.aborted) setBusy(false); }
  };
  const removeTask = async (task) => {
    if (!window.confirm(`Delete pending task "${task.name}"?`)) return;
    setBusy(true);
    try { await deleteRunTask(run.run_id, task.id); notify("Pending task deleted."); }
    catch (error) { notify(error.message, "error"); }
    finally { setBusy(false); }
  };
  const saveTask = async (toml, action) => {
    setBusy(true); setError(null);
    try {
      if (action === "save") {
        await editRunTask(run.run_id, editor.taskId, toml, editor.value);
        notify("Pending task updated.");
      } else {
        const inserted = await addRunTasks(run.run_id, toml, { duplicate: action === "duplicate" });
        notify(action === "duplicate" ? `Created task ${inserted[0].name}.` : `Appended ${inserted.length} task(s).`);
      }
      setEditor(null);
    } catch (error) { setError(error.message); }
    finally { setBusy(false); }
  };
  return <TabLoadingScope>
    <Box sx={{ display: "flex", alignItems: "center", gap: 1, flexWrap: "wrap", mb: 1 }}>
      {run.parent_run_id == null && authenticated && <WorkerPoolControls run={run} workers={workers} disabled={disabled} notify={notify} />}
      {run.parent_run_id != null && <Alert severity="info" sx={{ flex: "1 1 400px", minWidth: 0, flexWrap: { xs: "wrap", sm: "nowrap" },
        "& .MuiAlert-message": { flex: 1, minWidth: 0 },
        "& .MuiAlert-action": { width: { xs: "100%", sm: "auto" }, justifyContent: "flex-end", ml: { xs: 0, sm: "auto" } },
      }} action={
        <Button color="inherit" onClick={() => onSelectRun?.(run.parent_run_id)}>Manage parent workers</Button>
      }>Workers are allocated by the parent. Its controller manages this task queue. Duplicate this run as a standalone run to change it.</Alert>}
      <Box sx={{ ml: "auto", display: "flex", justifyContent: "flex-end" }}>{runActions}</Box>
    </Box>
    <WorkspaceTabs id="run" label="Run sections" value={view} onChange={setView}
      tabs={[["tasks", "Tasks"], ["summary", "Summary"], ["performance", "Performance"], ["settings", "Settings"]]} />
    <WorkspacePanel id="run" value="tasks" active={view === "tasks"}>
    <LoadingStatus loading={tasksLoading} label="Loading task queue…" />
    <LoadingStatus loading={busy && !editor} label="Loading task definition…" />
    {tasksError && <Alert severity="error">{tasksError.message}</Alert>}
    {isIntegration && !tasksLoading && <TaskQueuePanel tasks={taskList} selectedTaskId={selectedTask?.id} onSelectTask={selectTask}
      actions={editable && <Button size="small" variant="outlined" disabled={disabled || busy}
        onClick={() => { setError(null); setEditor({ title: "Add tasks", value: "" }); }}>Add tasks</Button>}
      renderTaskActions={(task) => <DefinitionButtons
        openLabel={`${editable && task.state === "pending" ? "Edit" : "Open"} task ${task.name}`}
        editable={editable && task.state === "pending"} disabled={disabled || busy} onOpen={() => openTask(task)}
        deleteLabel={`Delete task ${task.name}`} onDelete={authenticated ? () => removeTask(task) : null}
        deleteReason={!editable ? "This queue is managed by its parent run." : task.state !== "pending" ? "Only pending tasks can be deleted." : null} />} />}
    {!tasksLoading && (selectedTask || isIntegration) && <StableTaskOutput loading={taskOutput.isLoading} source={selectedTask?.id}>
    <TaskOutputPanel title={isIntegration ? "Selected task output" : "Run output"}
      key={selectedTask?.id ?? "empty"} runId={run.run_id} task={selectedTask} output={taskOutput} onSelectRun={onSelectRun}
      view={taskTab} onViewChange={setTaskTab} />
    </StableTaskOutput>}
    </WorkspacePanel>
    <WorkspacePanel id="run" value="summary" active={view === "summary"}>
      <RunInfo runId={run.run_id} section="summary" refreshKey={taskList.map((task) => `${task.id}:${task.state}`).join(",")}>
        <TaskSummary task={selectedTask} output={taskOutput} />
      </RunInfo>
    </WorkspacePanel>
    <WorkspacePanel id="run" value="performance" active={view === "performance"}>
      <PerformanceWorkspace runs={runs} workers={workers} selectedRun={run.run_id} isConnected={isConnected} tasks={taskList}
        view={navigation?.performanceTab} onViewChange={onNavigate ? (performanceTab) => onNavigate({ performanceTab }) : undefined} />
    </WorkspacePanel>
    <WorkspacePanel id="run" value="settings" active={view === "settings"}>
      <RunInfo runId={run.run_id} section="settings" refreshKey={taskList.map((task) => `${task.id}:${task.state}`).join(",")} />
    </WorkspacePanel>
    {editor && <TomlActionDialog open title={editor.title} label="Task TOML"
      submitLabel={editor.taskId ? "Save changes" : "Append tasks"} initialValue={editor.value}
      submitDisabled={Boolean(editor.taskId) && !canSave} exportName={editor.name || "task"}
      helperText={editor.taskId
        ? run.parent_run_id != null ? "This queue is managed by its parent. You can edit this draft for export or save it as a template. Open the run definition to create a standalone duplicate."
          : !authenticated ? "Sign in to save or duplicate tasks. You can edit this draft for copying or downloading."
          : canSave ? "Save changes to this pending task, or duplicate the draft at the end of the queue. Copy and download use the current text. Duplicates receive a name suffix if needed."
            : "Changes can only be saved as a new task. Duplicate appends this draft with a name suffix if needed. Copy and download use the current text."
        : "Append to the end of this run's queue. Tasks execute when reached if workers are assigned. Omitted sources use the previous state; named sources still refer to earlier tasks in this run."}
      warningText={editor.taskId && editable ? EXTERNAL_PATH_WARNING : null}
      templateKind="tasks" allowTemplateDelete
      templateSelectionStorageKey="dialogs.add_tasks.selected_template" busy={busy} error={error}
      onClose={() => { if (!busy) setEditor(null); }} onTemplateSaved={(_, name) => notify(`Saved task template "${name}".`)}
      onTemplateDeleted={(name) => notify(`Deleted task template "${name}".`)}
      onSubmit={editable ? (toml) => saveTask(toml, editor.taskId ? "save" : "append") : null}
      onDuplicate={editor.taskId && editable ? (toml) => saveTask(toml, "duplicate") : null} />}
  </TabLoadingScope>;
}

export default function RunsWorkspace({ runs, selectedRun, setSelectedRun, showChildRuns, setShowChildRuns,
  isConnected, onRunCreated, onSelectRun, hasMoreRuns, loadMoreRuns, isLoadingMoreRuns, isLoadingRuns,
  workers = [], navigation, onNavigate }) {
  const active = useWorkspaceActive();
  const { authenticated } = useAuth();
  const [editor, setEditor] = useState(null);
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState(null);
  const [notice, setNotice] = useState(null);
  const run = runs.find((run) => run.run_id === selectedRun);
  const notify = (message, severity = "success") => setNotice({ message, severity });
  const currentRunId = useRef(selectedRun);
  currentRunId.current = selectedRun;
  const currentEditor = useRef(editor);
  currentEditor.current = editor;
  const loading = useRef(null);
  // Run-bound drafts follow navigation; a new-run draft has no selected-run
  // destination and must survive polling/deletion selecting another run.
  useEffect(() => {
    if (!currentEditor.current || currentEditor.current.duplicate) {
      setEditor(null); setError(null);
      setBusy(false);
    }
    loading.current?.abort();
    return () => loading.current?.abort();
  }, [selectedRun]);
  const loadDefinition = async () => {
    setBusy(true); setError(null);
    const request = new AbortController();
    loading.current?.abort();
    loading.current = request;
    try {
      const { toml } = await fetchRunDefinition(run.run_id, false, request.signal);
      if (request.signal.aborted) return;
      setEditor({ title: `Run definition: ${run.run_name}`, name: run.run_name, value: toml,
        duplicate: true, standalone: run.parent_run_id != null });
    } catch (error) { if (!request.signal.aborted) notify(error.message, "error"); }
    finally { if (!request.signal.aborted) setBusy(false); }
  };
  const remove = async () => {
    if (!window.confirm("Delete this run and its child runs? This cannot be undone.")) return;
    const runId = run.run_id;
    setDeleting(true);
    try {
      await deleteRun(runId);
      if (currentRunId.current === runId) setSelectedRun?.(null);
      notify("Run deleted.");
    } catch (error) { notify(error.message, "error"); }
    finally { setDeleting(false); }
  };
  return <>
    <RunScopedWorkspace runs={runs} selectedRun={selectedRun} setSelectedRun={setSelectedRun}
      showChildRuns={showChildRuns} setShowChildRuns={setShowChildRuns} hasMoreRuns={hasMoreRuns}
      loadMoreRuns={loadMoreRuns} isLoadingMoreRuns={isLoadingMoreRuns} isLoadingRuns={isLoadingRuns}
      noRunsMessage="Create a run to start monitoring task output and engine configuration."
      noSelectionMessage="Pick a run to view task output and configuration."
      headerActions={<Box sx={{ display: "flex", justifyContent: "flex-end", gap: 1 }}>
        {authenticated && <Button variant="outlined" disabled={busy || deleting}
          onClick={() => { setError(null); setEditor({ title: "New run", value: "" }); }}>New run</Button>}

      </Box>}>
      <LoadingStatus loading={busy && !editor} label="Loading run definition…" />
      {deleting && <Alert severity="info" sx={{ mb: 2 }}>Deleting this run and its child runs. Large histories can take several minutes.</Alert>}
      {run && <RunContent key={run.run_id} run={run} disabled={busy || deleting} notify={notify}
        onSelectRun={onSelectRun} runs={runs} workers={workers} isConnected={isConnected} navigation={navigation} onNavigate={onNavigate}
        runActions={<DefinitionButtons openLabel="Open run definition" onOpen={loadDefinition} disabled={busy || deleting}
          deleteLabel="Delete run" onDelete={authenticated ? remove : null}
          deleteReason={run.parent_run_id != null ? "Delete managed children through their parent run." : null} />} />}
    </RunScopedWorkspace>
    {editor && <TomlActionDialog open title={editor.title} label="Run TOML"
      submitLabel={editor.duplicate ? editor.standalone ? "Create standalone run" : "Create duplicate run" : "Create run"}
      exportName={editor.name || "run"}
      initialValue={editor.value} templateKind="runs" allowTemplateDelete
      templateSelectionStorageKey="dialogs.create_run.selected_template" busy={busy} error={error}
      helperText={editor.duplicate
        ? "Create a fresh run from the current draft, with a name suffix if needed. Copy and download also use the current text. Results, trained state and workers are not copied; campaigns create new children."
        : "Create a fresh run from this definition. Copy and download use the current text."}
      warningText={editor.duplicate ? EXTERNAL_PATH_WARNING : null}
      onClose={() => { if (!busy) setEditor(null); }} onTemplateSaved={(_, name) => notify(`Saved run template "${name}".`)}
      onTemplateDeleted={(name) => notify(`Deleted run template "${name}".`)}
      onSubmit={authenticated ? async (toml) => {
        setBusy(true); setError(null);
        try {
          const created = await createRun(toml, { duplicate: Boolean(editor.duplicate) });
          setEditor(null); notify(`Created run ${created.run_name}.`);
          onRunCreated?.(Number(created.run_id));
        } catch (error) { setError(error.message); }
        finally { setBusy(false); }
      } : null} />}
    <Snackbar open={active && Boolean(notice)} autoHideDuration={notice?.severity === "error" ? null : 4000}
      onClose={(_, reason) => { if (reason !== "clickaway") setNotice(null); }} message={notice?.message || ""}
      action={<Button color="inherit" size="small" onClick={() => setNotice(null)}>Dismiss</Button>} />
  </>;
}
