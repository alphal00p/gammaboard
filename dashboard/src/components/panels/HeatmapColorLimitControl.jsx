import { useState } from "react";
import { Slider, Stack, TextField } from "@mui/material";
import { formatScientific } from "../../utils/formatters";

const HeatmapColorLimitControl = ({ side, limit, defaultScale, otherLimit, zeroOrigin, onChange }) => {
  const [draft, setDraft] = useState(null);
  const [error, setError] = useState(false);
  const isMin = side === "min";
  const label = isMin ? "Min" : "Max";
  const extent = isMin
    ? -defaultScale.zmin || defaultScale.zmax || 1
    : defaultScale.zmax || -defaultScale.zmin || 1;
  const span = zeroOrigin ? Math.max(extent * 2, Math.abs(limit)) : defaultScale.zmax - defaultScale.zmin;
  const sliderMin = zeroOrigin
    ? (isMin ? -span : 0)
    : (isMin ? Math.min(defaultScale.zmin - span, limit) : otherLimit);
  const sliderMax = zeroOrigin
    ? (isMin ? 0 : span)
    : (isMin ? otherLimit : Math.max(defaultScale.zmax + span, limit));
  const commit = (next) => {
    const valid = Number.isFinite(next)
      && (isMin ? next < otherLimit : next > otherLimit)
      && (!zeroOrigin || (isMin ? next <= 0 : next >= 0));
    if (valid) onChange(next);
    return valid;
  };
  return (
    <Stack direction="row" spacing={2} alignItems="center" sx={{ flex: "1 1 240px", maxWidth: 380 }}>
      <TextField
        label={label}
        type="number"
        size="small"
        value={draft ?? Number(limit.toPrecision(7))}
        error={error}
        helperText={error ? (isMin
          ? (zeroOrigin ? "Must be ≤ 0 and below Max." : "Must be below Max.")
          : (zeroOrigin ? "Must be ≥ 0 and above Min." : "Must be above Min.")) : null}
        onChange={(event) => {
          setDraft(event.target.value);
          setError(false);
        }}
        onBlur={() => {
          if (draft == null) return;
          if (draft.trim() && commit(Number(draft))) {
            setDraft(null);
            setError(false);
          } else {
            setError(true);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.target.blur();
          if (event.key === "Escape") {
            setDraft(null);
            setError(false);
          }
        }}
        slotProps={{ htmlInput: {
          "aria-label": `${label} color limit`,
          step: "any",
          ...(zeroOrigin ? (isMin ? { max: 0 } : { min: 0 }) : {}),
        } }}
        sx={{ width: 130, flexShrink: 0 }}
      />
      <Slider
        size="small"
        aria-label={`${label} color limit slider`}
        min={sliderMin}
        max={sliderMax}
        step={(sliderMax - sliderMin) / 1000}
        value={limit}
        onChange={(_event, next) => {
          setDraft(null);
          setError(false);
          commit(Number(next));
        }}
        valueLabelDisplay="auto"
        valueLabelFormat={(next) => formatScientific(next, 3)}
        sx={{ minWidth: 90 }}
      />
    </Stack>
  );
};

export default HeatmapColorLimitControl;
