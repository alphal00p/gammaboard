import { Alert, Box, Button, Card, CardContent, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useMemo, useState } from "react";

const FORM_REFRESH_HOLD_MS = 5000;

const QUEUE_TUNING_FIELDS = [
  { key: "target_batch_eval_ms", label: "Target Evaluation Time (ms)", kind: "float" },
  { key: "max_batch_size", label: "Maximum Evaluator Batch Size", kind: "int" },
  { key: "fixed_batch_size", label: "Fixed Evaluator Batch Size (optional)", kind: "int", optional: true },
];

const valueText = (value) => (value == null ? "" : String(value));

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
  const editableState = task?.state == null || ["pending", "active"].includes(task.state);

  const initialForm = useMemo(() => {
    const defaults = run?.queue_tuning_defaults ?? {};
    const override = isSampleTask ? task?.queue_tuning ?? null : null;
    const next = {};
    for (const field of QUEUE_TUNING_FIELDS) {
      const value = override?.[field.key] ?? defaults?.[field.key];
      next[field.key] = valueText(value);
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
                Evaluator batches adapt to the target duration. The sampler controls generation size; the queue refills below one pending batch per active evaluator.
              </Typography>
            </Box>
            {!task ? (
              <Alert severity="info">Select a task to tune queue settings.</Alert>
            ) : !isSampleTask ? (
              <Alert severity="info">Queue tuning is only supported for sample tasks.</Alert>
            ) : !authenticated ? (
              <Alert severity="info">Log in to update queue tuning.</Alert>
            ) : managedByParent ? (
              <Alert severity="info">This task queue is managed by its parent run.</Alert>
            ) : !editableState ? (
              <Alert severity="info">Queue tuning can only change pending or active tasks.</Alert>
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
                    <TextField
                      disabled={disabled}
                      key={field.key}
                      size="small"
                      label={field.label}
                      helperText={field.optional ? "Blank uses the run default." : undefined}
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
                    );
                    return field.optional ? (
                      <Box component="details" key={field.key} sx={{ gridColumn: "1 / -1" }}>
                        <Typography component="summary" sx={{ cursor: "pointer", mb: 1 }}>Advanced</Typography>
                        {input}
                      </Box>
                    ) : input;
                  })}
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
                    disabled={disabled}
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
