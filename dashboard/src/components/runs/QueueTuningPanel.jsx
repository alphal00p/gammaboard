import { Alert, Box, Button, Card, CardContent, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";

const FORM_REFRESH_HOLD_MS = 5000;

const QUEUE_TUNING_FIELDS = [
  { key: "queue_buffer", label: "Queue Buffer", kind: "float" },
  { key: "target_batch_eval_ms", label: "Target Batch Eval (ms)", kind: "float" },
  { key: "batch_size_deadband_ratio", label: "Batch Deadband Ratio", kind: "float" },
  { key: "batch_size_cooldown_ticks", label: "Batch Cooldown Ticks", kind: "int" },
  { key: "pending_refill_low_ratio", label: "Pending Refill Low Ratio", kind: "float" },
  { key: "pending_refill_high_ratio", label: "Pending Refill High Ratio", kind: "float" },
  { key: "max_batch_size", label: "Max Batch Size", kind: "int" },
  { key: "local_pending_buffer_multiplier", label: "Local Pending Multiplier", kind: "float" },
  { key: "max_queue_size", label: "Max Queue Size", kind: "int" },
  { key: "max_batches_per_tick", label: "Max Batches Per Tick", kind: "int" },
  { key: "max_insert_bundle_size", label: "Max Insert Bundle Size", kind: "int" },
  { key: "max_concurrent_insert_tasks", label: "Max Concurrent Insert Tasks", kind: "int" },
  { key: "completed_batch_fetch_limit", label: "Completed Batch Fetch Limit", kind: "int" },
];

const valueText = (value) => (value == null ? "" : String(value));

const parseFieldValue = (value, kind) => {
  const text = String(value ?? "").trim();
  if (!text) return { ok: false, value: null };
  const parsed = Number(text);
  if (!Number.isFinite(parsed)) return { ok: false, value: null };
  if (kind === "int" && !Number.isInteger(parsed)) return { ok: false, value: null };
  return { ok: true, value: parsed };
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
  const isSampleTask = task?.task?.kind === "sample";

  const initialForm = useMemo(() => {
    const defaults = run?.queue_tuning_defaults ?? {};
    const override = isSampleTask ? task?.task?.queue_tuning ?? null : null;
    const next = {};
    for (const field of QUEUE_TUNING_FIELDS) {
      next[field.key] = valueText(override?.[field.key] ?? defaults?.[field.key] ?? "");
    }
    return next;
  }, [isSampleTask, run, task]);

  const [form, setForm] = useState(initialForm);
  const [error, setError] = useState(null);
  const [refreshHoldUntilMs, setRefreshHoldUntilMs] = useState(0);
  const [skipNextExternalRefreshes, setSkipNextExternalRefreshes] = useState(0);

  useEffect(() => {
    if (Date.now() < refreshHoldUntilMs) return;
    if (skipNextExternalRefreshes > 0) {
      setSkipNextExternalRefreshes((count) => Math.max(0, count - 1));
      return;
    }
    setForm(initialForm);
    setError(null);
  }, [initialForm, refreshHoldUntilMs, skipNextExternalRefreshes]);

  const disabled = busy || !authenticated || runId == null || !task?.id || !isSampleTask;

  const handleSave = async () => {
    if (!onSave || disabled) return;
    const payload = {};
    for (const field of QUEUE_TUNING_FIELDS) {
      const parsed = parseFieldValue(form[field.key], field.kind);
      if (!parsed.ok) {
        setError(`Invalid value for "${field.label}".`);
        return false;
      }
      payload[field.key] = parsed.value;
    }
    setError(null);
    await onSave(payload);
    return true;
  };

  const handleClear = async () => {
    if (!onClear || disabled) return;
    setError(null);
    await onClear();
    return true;
  };

  return (
    <Box sx={{ mb: 3 }}>
      <Card variant="outlined">
        <CardContent>
          <Stack spacing={2}>
            <Box>
              <Typography variant="h6">Queue Tuning</Typography>
              <Typography variant="body2" color="text.secondary">
                Live task-level sampler queue tuning for the selected sample task.
              </Typography>
            </Box>
            {!task ? (
              <Alert severity="info">Select a task to tune queue settings.</Alert>
            ) : !isSampleTask ? (
              <Alert severity="info">Queue tuning is only supported for sample tasks.</Alert>
            ) : !authenticated ? (
              <Alert severity="info">Log in to update queue tuning.</Alert>
            ) : (
              <>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: { xs: "1fr", md: "repeat(3, minmax(0, 1fr))" },
                    gap: 1.5,
                  }}
                >
                  {QUEUE_TUNING_FIELDS.map((field) => (
                    <TextField
                      key={field.key}
                      size="small"
                      label={field.label}
                      value={form[field.key] ?? ""}
                      onChange={(event) => {
                        const raw = event.target.value;
                        const nextValue =
                          field.kind === "int"
                            ? raw.replace(/[^\d]/g, "")
                            : raw.replace(/[^0-9.-]/g, "");
                        setRefreshHoldUntilMs(Date.now() + FORM_REFRESH_HOLD_MS);
                        setForm((prev) => ({ ...prev, [field.key]: nextValue }));
                      }}
                    />
                  ))}
                </Box>
                {error ? <Alert severity="error">{error}</Alert> : null}
                <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                  <Button
                    variant="contained"
                    onClick={async () => {
                      const applied = await handleSave();
                      if (applied) {
                        setSkipNextExternalRefreshes(1);
                        setRefreshHoldUntilMs(0);
                      }
                    }}
                    disabled={busy}
                  >
                    Apply
                  </Button>
                  <Button
                    variant="outlined"
                    color="warning"
                    onClick={async () => {
                      const cleared = await handleClear();
                      if (cleared) {
                        setSkipNextExternalRefreshes(1);
                        setRefreshHoldUntilMs(0);
                      }
                    }}
                    disabled={busy}
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
