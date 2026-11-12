import { Alert, Autocomplete, Box, Button, Checkbox, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useMemo, useState } from "react";
import { useAuth } from "../../auth/AuthProvider";
import { updateCampaignTuning } from "../../services/api";
import LoadingStatus from "../common/LoadingStatus";
import DisclosureField from "../common/DisclosureField";

const fields = [
  ["target_batch_eval_ms", "Target evaluation time (ms)", "Adaptive evaluator batches aim for this duration."],
  ["max_generation_size", "Maximum sampler draw size", "Applies to the next draw; training windows are unchanged."],
  ["max_batch_size", "Maximum evaluator batch size", "Upper bound on samples in each evaluator batch."],
  ["fixed_batch_size", "Fixed evaluator batch size", "Optional override of adaptive batching. Clear an edited field to use its child run default."],
];
const matches = (task, scope) => scope === "all" || (scope === "publishing" && task.publish_result) || (scope === "training" && !task.publish_result) || scope === `name:${task.name}`;
export default function CampaignQueueTuning({ runId, taskId, resource }) {
  const { authenticated } = useAuth();
  const { data } = resource;
  const [selection, setSelection] = useState(null);
  const [scope, setScope] = useState("all");
  const [patch, setPatch] = useState({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState(null);
  const children = useMemo(() => data?.children ?? [], [data]);
  const selected = children.filter((child) => selection == null || selection.includes(child.label));
  const eligible = selected.flatMap((child) => (child.tasks ?? []).filter((task) => matches(task,scope) && ["pending","active"].includes(task.state)).map((task) => ({child,task})));
  const skipped = selected.flatMap((child) => (child.tasks ?? []).filter((task) => matches(task,scope) && !["pending","active"].includes(task.state)));
  const planned = selected.filter((child) => child.run_id == null).length;
  const editable = authenticated && ["pending","active"].includes(data?.state);
  const preview = (key) => {
    const values = eligible.map(({child,task}) => task.queue_tuning?.[key] ?? child.defaults?.[key] ?? null);
    return values.length && values.every((v) => v === values[0]) ? String(values[0] ?? "Run default") : "Mixed / unavailable";
  };
  const save = async () => {
    const values = {};
    for (const [key,raw] of Object.entries(patch)) {
      if (!raw.trim() && key === "fixed_batch_size") { values[key] = null; continue; }
      const value = Number(raw);
      if (!Number.isFinite(value) || value <= 0 || (key !== "target_batch_eval_ms" && !Number.isSafeInteger(value))) {setMessage({severity:"error",text:`Invalid ${fields.find(([k])=>k===key)[1].toLowerCase()}.`});return;}
      values[key] = value;
    }
    setBusy(true); setMessage(null);
    try {
      const result = await updateCampaignTuning(runId,taskId,{children:selected.map((c)=>c.label),scope,patch:values});
      setMessage({severity:"success",text:`Updated ${result.updated.length} task(s); skipped ${result.skipped.length} finished task(s). Saved for matching future child tasks.`});
      setPatch({}); resource.refresh();
    } catch (error) {setMessage({severity:"error",text:error.message});}
    finally {setBusy(false);}
  };
  return <Stack spacing={2}>
    <LoadingStatus loading={resource.isFetching || busy} label={busy ? "Applying queue tuning…" : "Loading child settings…"} />
    {resource.error && <Alert severity="error">{resource.error.message}</Alert>}
    {message && <Alert severity={message.severity}>{message.text}</Alert>}
    <Autocomplete multiple limitTags={3} disableCloseOnSelect options={children} value={selected} getOptionLabel={(child) => child.label}
      isOptionEqualToValue={(a,b) => a.label === b.label} disabled={busy}
      onChange={(_,next) => setSelection(next.map((c)=>c.label))}
      renderOption={(props,child,{selected}) => {const {key,...rest}=props; return <li key={key} {...rest}><Checkbox checked={selected} />{child.label}{child.run_id == null ? " · planned" : ` · #${child.run_id}`}</li>;}}
      renderInput={(params) => <TextField {...params} label="Apply to child runs" placeholder="Search children" />} />
    <Stack direction="row" spacing={1}><Button size="small" disabled={busy} onClick={()=>setSelection(null)}>Select all</Button><Button size="small" disabled={busy} onClick={()=>setSelection([])}>Deselect all</Button></Stack>
    <TextField select size="small" label="Task scope" value={scope} disabled={busy} onChange={(e)=>setScope(e.target.value)}>
      <MenuItem value="all">All unfinished sample tasks</MenuItem><MenuItem value="publishing">Publishing / inference tasks</MenuItem><MenuItem value="training">Non-publishing / training tasks</MenuItem>
      {[...new Set(children.flatMap((c)=>(c.tasks??[]).map((t)=>t.name)))].map((name)=><MenuItem key={name} value={`name:${name}`}>{name}</MenuItem>)}
    </TextField>
    <Typography variant="body2">{selected.length} children selected · {eligible.length} pending/active tasks · {skipped.length} finished tasks skipped · {planned} planned children</Typography>
    {!editable && <Alert severity="info">{authenticated ? "Queue tuning can only change pending or active campaigns." : "Log in to apply queue tuning."}</Alert>}
    {editable && !eligible.length && !planned && <Alert severity="info">Queue tuning can only change pending or active tasks.</Alert>}
    <Box sx={{display:"grid",gridTemplateColumns:{xs:"1fr",md:"1fr 1fr"},gap:2}}>
      {fields.map(([key,label,help])=><Box key={key}>
        <DisclosureField section label={label}><Typography variant="body2">{help}</Typography></DisclosureField>
        <TextField fullWidth size="small" type="number" disabled={!editable || busy} value={patch[key] ?? ""} placeholder={preview(key)}
          inputProps={{"aria-label":label,step:key === "target_batch_eval_ms" ? "any" : 1}}
          helperText={key in patch ? "Will be applied" : `Current: ${preview(key)} · unchanged`}
          onChange={(e)=>setPatch((old)=>({...old,[key]:e.target.value}))} />
      </Box>)}
    </Box>
    <Typography variant="body2" color="text.secondary">Only edited fields are applied. Overrides persist for the selected child names and task scope, including children created later. Existing batches finish with their original sizes.</Typography>
    <Stack direction="row" spacing={1}><Button variant="contained" disabled={!editable || busy || !selected.length || !Object.keys(patch).length || (!eligible.length && !planned)} onClick={save}>Apply to selected children</Button>
      <Button disabled={busy} onClick={()=>{setPatch({});setMessage(null);}}>Discard edits</Button></Stack>
  </Stack>;
}
