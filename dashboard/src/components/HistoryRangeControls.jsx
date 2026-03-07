import { Stack, TextField } from "@mui/material";
import { useEffect, useState } from "react";

const parseRelativeIndex = (raw) => {
  if (typeof raw !== "string") return null;
  const text = raw.trim();
  if (text.length === 0) return null;
  if (!/^-?\d+$/.test(text)) return null;
  const parsed = Number.parseInt(text, 10);
  if (!Number.isFinite(parsed)) return null;
  return parsed;
};

export default function HistoryRangeControls({ historyRange, setHistoryRange }) {
  const [draft, setDraft] = useState({
    start: String(historyRange.start),
    end: String(historyRange.end),
  });

  useEffect(() => {
    setDraft({
      start: String(historyRange.start),
      end: String(historyRange.end),
    });
  }, [historyRange.start, historyRange.end]);

  const setFieldDraft = (field, value) => {
    setDraft((prev) => ({ ...prev, [field]: value }));
  };

  const commitField = (field) => {
    const parsed = parseRelativeIndex(draft[field]);
    const previous = historyRange[field];

    if (parsed == null) {
      setFieldDraft(field, String(previous));
      return;
    }

    setHistoryRange((prev) => ({ ...prev, [field]: parsed }));
    setFieldDraft(field, String(parsed));
  };

  const handleFieldKeyDown = (event) => {
    if (event.key !== "Enter") return;
    event.currentTarget.blur();
  };

  return (
    <Stack direction={{ xs: "column", sm: "row" }} spacing={1.5} sx={{ mb: 2 }}>
      <TextField
        size="small"
        type="text"
        label="History Start"
        value={draft.start}
        onChange={(event) => setFieldDraft("start", event.target.value)}
        onBlur={() => commitField("start")}
        onKeyDown={handleFieldKeyDown}
        inputProps={{ inputMode: "numeric", pattern: "^-?\\d+$" }}
        helperText="Inclusive index (>=1 absolute id, negative = relative to newest)"
      />
      <TextField
        size="small"
        type="text"
        label="History End"
        value={draft.end}
        onChange={(event) => setFieldDraft("end", event.target.value)}
        onBlur={() => commitField("end")}
        onKeyDown={handleFieldKeyDown}
        inputProps={{ inputMode: "numeric", pattern: "^-?\\d+$" }}
        helperText="Inclusive index (>=1 absolute id, default -1 = newest)"
      />
    </Stack>
  );
}
