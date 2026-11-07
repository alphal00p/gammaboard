import { useEffect, useState } from "react";
import { TextField } from "@mui/material";

export default function RebinControl({ value, max, onChange, label = "Merge adjacent bins" }) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState(false);
  useEffect(() => { setDraft(String(value)); setError(false); }, [value]);
  const apply = (input, validate = true) => {
    const next = Number(input);
    if (!Number.isSafeInteger(next) || next < 1 || next > max) { setError(validate); return; }
    setError(false);
    if (next !== value) onChange(next);
  };
  return <TextField size="small" type="number" label={label} value={draft}
    inputProps={{ min: 1, max, step: 1 }} sx={{ width: 180 }} error={error}
    helperText={error ? `Use an integer from 1 to ${max}.` : undefined}
    onChange={(event) => {
      const input = event.target.value;
      setDraft(input);
      apply(input, false);
    }} onBlur={() => apply(draft)}
    onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); apply(draft); } }} />;
}
