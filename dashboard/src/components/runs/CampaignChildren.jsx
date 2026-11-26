import { lazy, useEffect, useMemo, useState } from "react";
import { Alert, Autocomplete, Box, Button, Card, CardContent, Checkbox, Dialog, DialogActions, DialogContent, DialogTitle, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useAuth } from "../../auth/AuthProvider";
import { useCampaign } from "../../hooks/useCampaign";
import { applyCampaignEdit, fetchRunDefinition, fetchRuns, previewCampaignEdit } from "../../services/api";
import { clearResourceCache } from "../../services/resourceCache";
import { createRequestId } from "../../utils/requestId";
import LoadingStatus from "../common/LoadingStatus";
import StatusChip from "../common/StatusChip";
const TomlEditor = lazy(() => import("../common/TomlEditor"));

export default function CampaignChildren({ runId, taskId }) {
  const { authenticated } = useAuth();
  const resource = useCampaign({ runId, taskId, section: "children" });
  const [action, setAction] = useState("add_task");
  const [dialog, setDialog] = useState(false);
  const [toml, setToml] = useState("");
  const [taskName, setTaskName] = useState("");
  const [source, setSource] = useState(null);
  const [label, setLabel] = useState("");
  const [coefficient, setCoefficient] = useState("1");
  const [excluded, setExcluded] = useState([]);
  const [preview, setPreview] = useState(null);
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState(null);
  const [notice, setNotice] = useState(null);
  const [search, setSearch] = useState("");
  const [candidates, setCandidates] = useState([]);
  const [requestId, setRequestId] = useState(null);
  const data = resource.data;
  const editable = authenticated && ["pending", "active"].includes(data?.state);
  const children = useMemo(() => data?.children ?? [], [data]);
  const bulk = ["add_task", "remove_task"].includes(action);
  const rows = useMemo(() => preview?.children?.length ? preview.children : children.map((child) => ({ ...child, applicable: action !== "add_task" || !child.failed })), [preview, children, action]);
  const selected = rows.filter((child) => child.applicable && !excluded.includes(child.run_id));
  const invalidate = () => { setPreview(null); setError(null); setRequestId(null); };
  useEffect(() => {
    if (!dialog || action !== "adopt") return;
    const controller = new AbortController();
    const timeout = setTimeout(() => {
      fetchRuns({ search, limit: 100 }, controller.signal).then(({ items }) => {
        if (!controller.signal.aborted) setCandidates(items.filter((run) => run.run_id !== runId && run.parent_run_id == null && (!run.kind || run.kind === "integration")));
      }).catch((error) => { if (!controller.signal.aborted) setError(error.message); });
    }, 250);
    return () => { clearTimeout(timeout); controller.abort(); };
  }, [dialog, action, search, runId]);
  const open = (next) => { setAction(next); setDialog(true); setSource(null); setLabel(""); setTaskName(""); setPreview(null); setError(null); setNotice(null); setRequestId(null); };
  const payload = () => ({ action, toml, task_name: taskName, run_id: source?.run_id ?? null, label, coefficient: Number(coefficient), children: bulk ? selected.map((child) => child.run_id) : [] });
  const check = async () => {
    setBusy(true); invalidate();
    try {
      const nextId = createRequestId();
      const nextPreview = await previewCampaignEdit(runId, taskId, payload());
      setRequestId(nextId); setPreview(nextPreview);
    }
    catch (error) { setError(error.message); }
    finally { setBusy(false); }
  };
  const apply = async () => {
    if (!preview || !requestId) return;
    setBusy(true); setError(null);
    try {
      const result = await applyCampaignEdit(runId, taskId, { ...payload(), expected: preview, request_id: requestId });
      setNotice(result.queued ? "Requested. Waiting for the run to checkpoint and release its workers." : "Changes applied.");
      clearResourceCache(); setRefreshing(true);
      await resource.refresh();
      setDialog(false); setPreview(null); setTaskName("");
    } catch (error) { setError(error.message); }
    finally { setRefreshing(false); setBusy(false); }
  };
  const chooseTemplate = async (child) => {
    setSource(child); invalidate();
    if (!child) return;
    setBusy(true);
    try { const result = await fetchRunDefinition(child.run_id); setToml(result.toml); setLabel(`${child.label || child.name}-copy`); }
    catch (error) { setError(error.message); }
    finally { setBusy(false); }
  };
  const selectTargets = (next) => {
    setExcluded(rows.filter((child) => child.applicable && !next.some((s) => s.run_id === child.run_id)).map((child) => child.run_id));
    try { setRequestId(createRequestId()); setError(null); }
    catch (error) { invalidate(); setError(error.message); }
  };
  const targets = <Autocomplete multiple disableCloseOnSelect options={rows} value={selected}
    isOptionEqualToValue={(a,b) => a.run_id === b.run_id} getOptionLabel={(child) => child.label || child.name || `#${child.run_id}`}
    getOptionDisabled={(child) => !child.applicable} disabled={!editable || busy}
    onChange={(_, next) => selectTargets(next)}
    renderTags={() => <Typography sx={{ ml:1 }}>Target: {selected.length}/{children.length} children</Typography>}
    renderOption={(props, child, { selected }) => { const { key, ...rest } = props; return <li key={key} {...rest}><Checkbox checked={selected} /><Box sx={{flex:1}}>{child.label || child.name}{child.reason && <Typography variant="caption" display="block">{child.reason}</Typography>}</Box><StatusChip status={child.status} /></li>; }}
    renderInput={(params) => <TextField {...params} size="small" label="Target children" placeholder={selected.length ? "Search children" : `Target: 0/${children.length} children`} />} />;
  const title = { add_task:"Add task", remove_task:"Remove task", create:"Create child", delete:"Delete child", adopt:"Add existing run" }[action];
  return <Stack spacing={2}>
    <LoadingStatus loading={resource.isFetching || busy} label={refreshing ? "Refreshing children and tasks…" : busy ? "Checking child edits…" : "Loading children…"} />
    {resource.error && <Alert severity="error">{resource.error.message}</Alert>}
    {notice && <Alert severity="success">{notice}</Alert>}
    {!editable && data && <Alert severity="info">{authenticated ? "This campaign is no longer editable." : "Log in to edit children."}</Alert>}
    <Card variant="outlined"><CardContent><Stack spacing={2}>
      <Typography variant="subtitle1">Tasks across children</Typography>
      {!dialog && targets}
      <Stack direction="row" spacing={1}><Button variant="outlined" disabled={!editable || busy} onClick={() => open("add_task")}>Add task</Button><Button variant="outlined" disabled={!editable || busy} onClick={() => open("remove_task")}>Remove task</Button></Stack>
    </Stack></CardContent></Card>
    <Card variant="outlined"><CardContent><Stack spacing={2}>
      <Typography variant="subtitle1">Manage children</Typography>
      <Stack direction="row" spacing={1}><Button disabled={!editable || busy} color="error" onClick={() => open("delete")}>Delete</Button><Button disabled={!editable || busy} onClick={() => open("create")}>Create</Button><Button disabled={!editable || busy} onClick={() => open("adopt")}>Add existing</Button></Stack>
      {(data?.operations ?? []).filter((op) => op.state !== "completed").map((op) => <Alert key={op.id} severity={op.state === "failed" ? "error" : "info"}>Run #{op.run_id}: {op.operation} — {op.error || "waiting for workers to drain"}</Alert>)}
    </Stack></CardContent></Card>
    <Dialog open={dialog} onClose={() => { if (!busy) setDialog(false); }} fullWidth maxWidth="md">
      <DialogTitle>{title}</DialogTitle><DialogContent><Stack spacing={2} sx={{pt:1}}>
        {error && <Alert severity="error">{error}</Alert>}
        {bulk && targets}
        {action === "remove_task" && <TextField select label="Task name" value={taskName} onChange={(e) => { setTaskName(e.target.value); invalidate(); }}>
          {(data?.task_names ?? []).map((task) => <MenuItem key={task.name} value={task.name}>{task.name} — {task.count}/{children.length} children</MenuItem>)}
        </TextField>}
        {["create", "delete"].includes(action) && <Autocomplete options={children} value={source} disabled={busy}
          getOptionLabel={(child) => `${child.label || child.name} (#${child.run_id})`} isOptionEqualToValue={(a,b) => a.run_id === b.run_id}
          onChange={(_,child) => { if (action === "create") chooseTemplate(child); else { setSource(child); invalidate(); } }}
          renderOption={(props,child) => { const {key,...rest}=props; return <li key={key} {...rest}><Box sx={{flex:1}}>{child.label || child.name}</Box><StatusChip status={child.status} /></li>; }}
          renderInput={(params) => <TextField {...params} label={action === "create" ? "Start from child (optional)" : "Child run"} />} />}
        {action === "adopt" && <Autocomplete options={candidates} value={source} disabled={busy} filterOptions={(options) => options}
          getOptionLabel={(run) => `${run.run_name} (#${run.run_id})`} isOptionEqualToValue={(a,b) => a.run_id === b.run_id}
          onInputChange={(_,value,reason) => { if (reason === "input" || reason === "clear") setSearch(value); }}
          onChange={(_,run) => { setSource(run); setLabel(run?.run_name || ""); invalidate(); }}
          renderInput={(params) => <TextField {...params} label="Search standalone runs" />} />}
        {["create", "adopt"].includes(action) && <Stack direction="row" spacing={2}><TextField label="Child label" value={label} onChange={(e) => {setLabel(e.target.value);invalidate();}} /><TextField label="Coefficient" type="number" value={coefficient} onChange={(e) => {setCoefficient(e.target.value);invalidate();}} /></Stack>}
        {["create", "add_task"].includes(action) && <TomlEditor label={action === "create" ? "Run TOML" : "Task TOML"} value={toml} disabled={busy} onChange={(value) => {setToml(value);invalidate();}} />}
        {action === "create" && <Typography variant="body2">Creates a fresh run from this definition. Check sampler output paths before creating another run.</Typography>}
        {preview && <>
          {bulk ? <><Typography>Apply to {selected.length} children: {selected.map((child) => child.label).join(", ") || "None"}.</Typography>
            {rows.filter((child) => !child.applicable).map((child) => <Typography key={child.run_id} variant="body2">{child.label}: {child.reason}</Typography>)}</> :
            <Typography>{action === "delete" ? `Delete ${preview.source?.name}? This deletes the run and removes its contribution from future campaign totals.` : action === "adopt" ? `Adopt ${preview.source?.name}, including ${Number(preview.source?.produced_samples).toLocaleString()} existing samples, ${(Number(preview.source?.cpu_seconds || 0) / 3600).toPrecision(4)} CPU-hours, and its ${preview.source?.workers} assigned workers? Its run ID, tasks and results are retained.` : `Create child “${label}” with coefficient ${coefficient}?`}</Typography>}
        </>}
      </Stack></DialogContent><DialogActions>
        <Button disabled={busy} onClick={() => setDialog(false)}>Cancel</Button>
        <Button disabled={busy || !editable} onClick={check}>Check compatibility</Button>
        <Button variant="contained" color={action === "delete" || action === "remove_task" ? "error" : "primary"} disabled={busy || !editable || !preview || !requestId || (bulk && !selected.length)} onClick={apply}>Confirm {title.toLowerCase()}</Button>
      </DialogActions>
    </Dialog>
  </Stack>;
}
