import { Alert, Box, Button, Card, CardContent, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { Fragment, useEffect, useRef, useState } from "react";
import { useAuth } from "../../auth/AuthProvider";
import { useCampaign } from "../../hooks/useCampaign";
import { updateCampaignScheduling } from "../../services/api";
import LoadingStatus from "../common/LoadingStatus";
import DisclosureField from "../common/DisclosureField";
import { fieldHelp } from "../panels/fieldHelp";

const fields = [
  ["allocation", "max_active_runs", "Maximum concurrent children", true],
  ["allocation", "min_evaluators_per_run", "Minimum evaluators per child", true],
  ["allocation", "allocation_window_samples", "Allocation window (samples)", true],
  ["allocation", "min_samples_per_child", "Pilot minimum per child", true],
  ["stop_condition", "min_total_samples", "Minimum total samples", true],
  ["stop_condition", "max_total_samples", "Sample cap", true, true],
  ["stop_condition", "absolute_error", "Absolute error target", false, true],
  ["stop_condition", "relative_error", "Relative error target (fraction)", false, true],
];
const formOf = (settings) => Object.fromEntries(fields.map(([section,key]) => [key, settings?.[section]?.[key] == null ? "" : String(settings[section][key])]));
export default function CampaignScheduling({ runId, taskId }) {
  const resource = useCampaign({ runId, taskId });
  const { authenticated } = useAuth();
  const dirty = useRef(false);
  const expected = useRef(null);
  const [form, setForm] = useState({});
  const [algorithm, setAlgorithm] = useState("variance_reduction_rate");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState(null);
  const { data } = resource;
  useEffect(() => {
    if (!data || dirty.current) return;
    setForm(formOf(data.settings)); setAlgorithm(data.settings.allocation.algorithm); expected.current = data.settings;
  }, [data]);
  const editable = authenticated && ["pending", "active"].includes(data?.state);
  const reset = () => { dirty.current = false; setForm(formOf(data?.settings)); setAlgorithm(data?.settings?.allocation.algorithm ?? "variance_reduction_rate"); expected.current = data?.settings; setMessage(null); };
  const save = async () => {
    const settings = { allocation: { algorithm }, stop_condition: {} };
    for (const [section,key,label,integer,optional] of fields) {
      const raw = String(form[key] ?? "").trim();
      if (!raw && optional) { settings[section][key] = null; continue; }
      const value = raw ? Number(raw) : NaN;
      const allowZero = ["min_total_samples", "min_samples_per_child"].includes(key);
      if (!Number.isFinite(value) || (integer && !Number.isSafeInteger(value)) || (allowZero ? value < 0 : value <= 0)) {
        setMessage({ severity:"error", text:`Invalid ${label.toLowerCase()}.` }); return;
      }
      settings[section][key] = value;
    }
    setBusy(true); setMessage(null);
    try {
      await updateCampaignScheduling(runId, taskId, { expected: expected.current, ...settings });
      dirty.current = false; expected.current = settings; resource.refresh();
      setMessage({ severity:"success", text:"Scheduling updated. Existing grants are preserved." });
    } catch (error) { setMessage({ severity:"error", text:error.message }); }
    finally { setBusy(false); }
  };
  return <Stack spacing={2}>
    <LoadingStatus loading={resource.isFetching || busy} label={busy ? "Saving scheduling…" : "Loading scheduling…"} />
    {resource.error && <Alert severity="error">{resource.error.message}</Alert>}
    {message && <Alert severity={message.severity}>{message.text}</Alert>}
    {data && <>
      <Card variant="outlined"><CardContent><Stack spacing={1}>
        <Typography>Configured concurrency: {data.settings.allocation.max_active_runs} · Capacity with assigned workers: {data.capacity}</Typography>
        <Typography variant="body2">{data.samplers} samplers · {data.evaluators} evaluators · {Number(data.reserved_samples).toLocaleString()} reserved samples</Typography>
        <Typography variant="body2">Scheduled children: {(data.controller_output?.selected_child_run_ids ?? []).map((id) => data.children.find((child) => child.run_id === id)?.label ?? `#${id}`).join(", ") || "None"}</Typography>
        {data.capacity < data.settings.allocation.max_active_runs && <Alert severity="info">{data.samplers <= Math.floor(data.evaluators / data.settings.allocation.min_evaluators_per_run) ? "Sampler availability" : "Evaluator availability"} limits concurrency. Assign additional workers to the campaign to increase capacity.</Alert>}
        <Typography variant="body2" color="text.secondary">Applying settings immediately rechecks assignments. Workers checkpoint before switching; existing sample reservations are retained. New allocation windows apply to future grants. Training samples count toward the total budget.</Typography>
      </Stack></CardContent></Card>
      {!editable && <Alert severity="info">{authenticated ? "Completed or failed campaigns are read-only." : "Log in to change scheduling."}</Alert>}
      <Box sx={{ display:"grid", gridTemplateColumns:{xs:"1fr",md:"1fr 1fr"}, gap:2 }}>
        <TextField select size="small" sx={{gridColumn:"1 / -1"}} label="Allocation algorithm" disabled={!editable || busy} value={algorithm}
          onChange={(e) => { dirty.current = true; setAlgorithm(e.target.value); }}>
          <MenuItem value="variance_reduction_rate">Variance reduction per time</MenuItem><MenuItem value="largest_variance">Largest variance</MenuItem>
        </TextField>
        {fields.map(([,key,label,integer,optional], index) => <Fragment key={key}>
          {index === 4 && <Typography variant="subtitle1" sx={{gridColumn:"1 / -1"}}>Stopping conditions</Typography>}
          <Box>
          <DisclosureField section label={label}><Typography variant="body2">{fieldHelp(key,label,"task_summary")}</Typography></DisclosureField>
          <TextField fullWidth size="small" type="number" inputProps={{"aria-label":label, step:integer ? 1 : "any"}} disabled={!editable || busy}
            value={form[key] ?? ""} placeholder={optional ? "No limit" : undefined}
            onChange={(e) => { dirty.current = true; setForm((old) => ({...old,[key]:e.target.value})); }} />
        </Box></Fragment>)}
      </Box>
      <Stack direction="row" spacing={1}><Button variant="contained" disabled={!editable || busy || !dirty.current} onClick={save}>Apply scheduling</Button>
        <Button disabled={busy} onClick={reset}>Reload settings</Button></Stack>
    </>}
  </Stack>;
}
