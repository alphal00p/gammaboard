import DisclosureField from "../common/DisclosureField";
import { Alert, Box, Button, Card, CardContent, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";

const QUEUE_TUNING_FIELDS = [
  { key: "target_batch_eval_ms", label: "Target Evaluation Time (ms)", kind: "float", help: "Adaptive evaluator batches aim for this evaluation duration. Queue and database waits are measured separately." },
  { key: "max_generation_size", label: "Maximum Sampler Draw Size", kind: "int", help: "Applies to the next draw. Training windows are unchanged." },
  { key: "max_batch_size", label: "Maximum Evaluator Batch Size", kind: "int", help: "Upper bound on the number of points sent to an evaluator in one batch. A sampler draw can supply several evaluator batches." },
  { key: "fixed_batch_size", label: "Fixed Evaluator Batch Size (optional)", kind: "int", optional: true },
];

const valueText = (value) => (value == null ? "" : String(value));
const queueForm = (defaults = {}, override = {}) => Object.fromEntries(QUEUE_TUNING_FIELDS
  .map(({ key }) => [key, valueText(override?.[key] ?? defaults?.[key])]));

const parseFieldValue = (value, kind) => {
  const text = String(value ?? "").trim();
  if (!text) return { ok: false, value: null };
  const parsed = Number(text);
  if (!Number.isFinite(parsed)) return { ok: false, value: null };
  if (kind === "int" && !Number.isInteger(parsed)) return { ok: false, value: null };
  return { ok: parsed > 0, value: parsed };
};

const QueueTuningPanel = ({
  run = null,
  runId = null,
  task = null,
  authenticated = false,
  busy = false,
  onSave,
  onClear,
}) => {
  const isSampleTask = task?.is_sample === true;
  const managedByParent = run?.parent_run_id != null;
  const editableState = task?.state === "active";

  // Compare values, not polling object identities: an identical stale response
  // must not overwrite the form immediately after a successful save.
  const serverForm = JSON.stringify(queueForm(run?.queue_tuning_defaults, isSampleTask ? task?.queue_tuning : null));
  const dirty = useRef(false);
  const [form, setForm] = useState(() => JSON.parse(serverForm));
  const [error, setError] = useState(null);

  useEffect(() => {
    if (dirty.current) return;
    setForm(JSON.parse(serverForm));
    setError(null);
  }, [serverForm]);

  const disabled = busy || !authenticated || managedByParent || !editableState || runId == null || !task?.id || !isSampleTask;

  const handleSave = async () => {
    if (!onSave || disabled) return;
    const payload = {};
    for (const field of QUEUE_TUNING_FIELDS) {
      if (field.optional && !String(form[field.key] ?? "").trim()) continue;
      const parsed = parseFieldValue(form[field.key], field.kind);
      if (!parsed.ok) {
        setError(`Invalid value for "${field.label}".`);
        return false;
      }
      payload[field.key] = parsed.value;
    }
    setError(null);
    if (await onSave(payload) === false) return false;
    dirty.current = false;
    return true;
  };

  const handleClear = async () => {
    if (!onClear || disabled) return;
    setError(null);
    if (await onClear() === false) return false;
    dirty.current = false;
    setForm(queueForm(run?.queue_tuning_defaults));
    return true;
  };

  return (
    <Box sx={{ mb: 3 }}>
      <Card variant="outlined">
        <CardContent>
          <Stack spacing={2}>
            <Box>
              <Typography variant="body2" color="text.secondary">
                Evaluator batches adapt to the target duration. The draw limit bounds how many samples the sampler generates at once; each draw can supply multiple evaluator batches.
              </Typography>
            </Box>
            {!task ? (
              <Alert severity="info">No active sampling task.</Alert>
            ) : !isSampleTask ? (
              <Alert severity="info">Queue tuning is only supported for sample tasks.</Alert>
            ) : !authenticated ? (
              <Alert severity="info">Log in to update queue tuning.</Alert>
            ) : managedByParent ? (
              <Alert severity="info">This task queue is managed by its parent run.</Alert>
            ) : !editableState ? (
              <Alert severity="info">Queue tuning is available only for the active sampling task.</Alert>
            ) : (
              <>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: { xs: "1fr", md: "repeat(2, minmax(0, 1fr))" },
                    gap: 1.5,
                  }}
                >
                  {QUEUE_TUNING_FIELDS.map((field) => {
                    const input = (
                    <TextField fullWidth
                      disabled={disabled}
                      key={field.key}
                      size="small"
                      label={field.optional ? field.label : undefined}
                      slotProps={{ htmlInput: { "aria-label": field.label } }}
                      helperText={field.optional ? "Blank uses the run default." : undefined}
                      value={form[field.key] ?? ""}
                      onChange={(event) => {
                        const raw = event.target.value;
                        const nextValue =
                          field.kind === "int"
                            ? raw.replace(/[^\d]/g, "")
                            : raw.replace(/[^0-9.-]/g, "");
                        dirty.current = true;
                        setForm((prev) => ({ ...prev, [field.key]: nextValue }));
                      }}
                    />
                    );
                    return field.optional ? (
                      <DisclosureField key={field.key} label="Fixed evaluator batch size" value={form[field.key] || "Automatic"} sx={{ gridColumn: "1 / -1" }}>
                        <Typography variant="caption" component="p" sx={{ mb: 1 }}>Advanced override. Blank uses the run default; an explicit size overrides adaptive batching.</Typography>
                        {input}
                      </DisclosureField>
                    ) : <DisclosureField key={field.key} section label={field.label} ariaLabel={`${field.label} explanation`}
                      actions={<Box sx={{ width: { xs: 100, md: 130 } }}>{input}</Box>}>
                      <Typography variant="caption">{field.help}</Typography>
                    </DisclosureField>;
                  })}
                </Box>
                {error ? <Alert severity="error">{error}</Alert> : null}
                <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                  <Button
                    variant="contained"
                    onClick={handleSave}
                    disabled={disabled}
                  >
                    Apply
                  </Button>
                  <Button
                    variant="outlined"
                    color="warning"
                    onClick={handleClear}
                    disabled={disabled}
                  >
                    Clear Task Override
                  </Button>
                </Stack>
              </>
            )}
          </Stack>
        </CardContent>
      </Card>
    </Box>
  );
};

export default QueueTuningPanel;
