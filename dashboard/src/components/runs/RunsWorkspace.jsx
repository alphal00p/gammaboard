import { Alert, Box, Button, IconButton, Snackbar, Stack, TextField, Tooltip } from "@mui/material";
import DeleteOutlineIcon from "@mui/icons-material/DeleteOutline";
import DescriptionOutlinedIcon from "@mui/icons-material/DescriptionOutlined";
import EditOutlinedIcon from "@mui/icons-material/EditOutlined";
import { lazy, useEffect, useRef, useState } from "react";
import { useAuth } from "../../auth/AuthProvider";
import { useRunTasks } from "../../hooks/useRunTasks";
import { addRunTasks, autoAssignRun, createRun, deleteRun, deleteRunTask, editRunTask,
  fetchNodes, fetchRunDefinition, fetchTaskDefinition, pauseRun, unassignNode } from "../../services/api";
import { asArray } from "../../utils/collections";
import { getCurrentTask } from "../../utils/tasks";
import RunScopedWorkspace from "../common/RunScopedWorkspace";

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

function WorkerPoolControls({ run, disabled, notify }) {
  const [busy, setBusy] = useState(false);
  const [count, setCount] = useState(() => window.localStorage.getItem("runs.evaluator_count") || "");
  const act = async (action) => {
    setBusy(true);
    try {
      const target = count.trim() ? Number(count) : null;
      if (action !== "pause" && target != null && (!Number.isSafeInteger(target) || target < 1)) throw new Error("N must be at least 1.");
      if (count) window.localStorage.setItem("runs.evaluator_count", count);
      else window.localStorage.removeItem("runs.evaluator_count");
      if (action === "assign") {
        const result = await autoAssignRun(run.run_id, { maxEvaluators: target });
        notify(`Assigned ${(result.assigned_evaluators?.length || 0) + (result.assigned_sampler ? 1 : 0)} node(s); resumed ${result.resumed_nodes || 0} pool member(s).`);
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
  return <Stack direction={{ xs: "column", md: "row" }} spacing={1.5} sx={{ mb: 2 }}>
    <TextField size="small" value={count} placeholder="all" inputProps={{ "aria-label": "Evaluator node count" }}
      onChange={(event) => setCount(event.target.value.replace(/[^\d]/g, ""))} />
    <Button variant="contained" disabled={disabled || busy} onClick={() => act("assign")}>Assign / Resume</Button>
    <Button variant="outlined" color="warning" disabled={disabled || busy} onClick={() => act("remove")}>Remove evaluators</Button>
    <Button variant="outlined" color="warning" disabled={disabled || busy} onClick={() => act("pause")}>Pause Run</Button>
  </Stack>;
}

function RunContent({ run, disabled, notify, onSelectRun }) {
  const { authenticated } = useAuth();
  const { tasks } = useRunTasks(run.run_id, 2000);
  const [selectedId, setSelectedId] = useState(null);
  const loading = useRef(null);
  useEffect(() => () => loading.current?.abort(), []);
  const [editor, setEditor] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const isIntegration = !run.kind || run.kind === "integration";
  const editable = authenticated && isIntegration && run.parent_run_id == null;
  const taskList = asArray(tasks);
  const selectedTask = taskList.find((task) => task.id === selectedId) ?? getCurrentTask(taskList);
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
  return <>
    {run.parent_run_id != null ? <Alert severity="info" sx={{ mb: 2 }} action={
      <Button color="inherit" onClick={() => onSelectRun?.(run.parent_run_id)}>Manage parent workers</Button>
    }>Workers are allocated by the parent. Its controller manages this task queue. Duplicate this run as a standalone run to change it.</Alert>
      : authenticated && <WorkerPoolControls run={run} disabled={disabled} notify={notify} />}
    {isIntegration && <TaskQueuePanel tasks={taskList} selectedTaskId={selectedTask?.id} onSelectTask={setSelectedId}
      actions={editable && <Button size="small" variant="outlined" disabled={disabled || busy}
        onClick={() => { setError(null); setEditor({ title: "Add tasks", value: "" }); }}>Add tasks</Button>}
      renderTaskActions={(task) => <DefinitionButtons
        openLabel={`${editable && task.state === "pending" ? "Edit" : "Open"} task ${task.name}`}
        editable={editable && task.state === "pending"} disabled={disabled || busy} onOpen={() => openTask(task)}
        deleteLabel={`Delete task ${task.name}`} onDelete={authenticated ? () => removeTask(task) : null}
        deleteReason={!editable ? "This queue is managed by its parent run." : task.state !== "pending" ? "Only pending tasks can be deleted." : null} />} />}
    {(selectedTask || isIntegration) && <TaskOutputPanel title={isIntegration ? "Selected Task Output" : "Run Output"}
      key={selectedTask?.id ?? "empty"} runId={run.run_id} task={selectedTask} onSelectRun={onSelectRun} />}
    <RunInfo runId={run.run_id} />
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
  </>;
}

export default function RunsWorkspace({ runs, selectedRun, setSelectedRun, showChildRuns, setShowChildRuns,
  isConnected, serverName, onRunCreated, onSelectRun, hasMoreRuns, loadMoreRuns, isLoadingMoreRuns }) {
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
  const loading = useRef(null);
  // Navigation closes drafts and cancels reads instead of changing their destination.
  useEffect(() => {
    setEditor(null); setError(null);
    loading.current?.abort();
    setBusy(false);
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
      loadMoreRuns={loadMoreRuns} isLoadingMoreRuns={isLoadingMoreRuns} isConnected={isConnected} serverName={serverName}
      noRunsMessage="Create a run to start monitoring task output and engine configuration."
      noSelectionMessage="Pick a run to view task output and configuration."
      headerActions={<Box sx={{ display: "flex", justifyContent: "flex-end", gap: 1, mb: 2 }}>
        {authenticated && <Button variant="outlined" disabled={busy || deleting}
          onClick={() => { setError(null); setEditor({ title: "New run", value: "" }); }}>New run</Button>}
        {run && <DefinitionButtons openLabel="Open run definition" onOpen={loadDefinition} disabled={busy || deleting}
          deleteLabel="Delete run" onDelete={authenticated ? remove : null}
          deleteReason={run.parent_run_id != null ? "Delete managed children through their parent run." : null} />}
      </Box>}>
      {deleting && <Alert severity="info" sx={{ mb: 2 }}>Deleting this run and its child runs. Large histories can take several minutes.</Alert>}
      {run && <RunContent key={run.run_id} run={run} disabled={busy || deleting} notify={notify}
        onSelectRun={onSelectRun} />}
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
    <Snackbar open={Boolean(notice)} autoHideDuration={notice?.severity === "error" ? null : 4000}
      onClose={(_, reason) => { if (reason !== "clickaway") setNotice(null); }} message={notice?.message || ""}
      action={<Button color="inherit" size="small" onClick={() => setNotice(null)}>Dismiss</Button>} />
  </>;
}
