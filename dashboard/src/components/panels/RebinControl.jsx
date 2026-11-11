import { useEffect, useState } from "react";
import { Box, ButtonBase, TextField } from "@mui/material";
import ArrowDropUpIcon from "@mui/icons-material/ArrowDropUp";
import ArrowDropDownIcon from "@mui/icons-material/ArrowDropDown";

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
  const numericDraft = Number(draft);
  const stepBase = Number.isSafeInteger(numericDraft) && numericDraft >= 1 && numericDraft <= max ? numericDraft : value;
  const step = (delta) => {
    const next = Math.min(max, Math.max(1, stepBase + delta));
    setDraft(String(next));
    apply(next);
  };
  return <TextField size="small" type="number" label={label} value={draft}
    inputProps={{ min: 1, max, step: 1 }} sx={{ width: 180,
      "& input": { MozAppearance: "textfield", pr: 5 },
      "& input::-webkit-inner-spin-button, & input::-webkit-outer-spin-button": { WebkitAppearance: "none", margin: 0 },
    }} error={error}
    slotProps={{ input: { endAdornment: <Box data-rebin-stepper sx={{ position: "absolute", right: 1, top: 1, bottom: 1, width: 34,
      display: "grid", gridTemplateRows: "repeat(2, minmax(0, 1fr))", borderLeft: 1, borderColor: "divider" }}>
      <ButtonBase type="button" aria-label="Increase merged bins" disabled={stepBase >= max}
        onMouseDown={(event) => event.preventDefault()} onClick={() => step(1)}
        sx={{ borderBottom: 1, borderColor: "divider", "&:hover": { bgcolor: "action.hover" }, "&.Mui-disabled": { color: "text.disabled" }, "&.Mui-focusVisible": { bgcolor: "action.focus" } }}>
        <ArrowDropUpIcon fontSize="small" />
      </ButtonBase>
      <ButtonBase type="button" aria-label="Decrease merged bins" disabled={stepBase <= 1}
        onMouseDown={(event) => event.preventDefault()} onClick={() => step(-1)}
        sx={{ "&:hover": { bgcolor: "action.hover" }, "&.Mui-disabled": { color: "text.disabled" }, "&.Mui-focusVisible": { bgcolor: "action.focus" } }}>
        <ArrowDropDownIcon fontSize="small" />
      </ButtonBase>
    </Box> } }}
    helperText={error ? `Use an integer from 1 to ${max}.` : undefined}
    onChange={(event) => {
      const input = event.target.value;
      setDraft(input);
      apply(input, false);
    }} onBlur={() => apply(draft)}
    onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); apply(draft); } }} />;
}
