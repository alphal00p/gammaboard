import { useRef, useState } from "react";
import { Autocomplete, Box, Chip, CircularProgress, FormControlLabel, Switch, TextField, Typography } from "@mui/material";
import { formatRunLabel, formatRunSecondaryLabel, orderRunsForSelector } from "../utils/runs";
import { useRunSearch } from "../hooks/useRunSearch";
import StatusChip from "./common/StatusChip";

const MORE = { loadMore: true };
const RunSelector = ({
  runs, selectedRun, onRunChange, showChildRuns = false, onShowChildRunsChange = null,
  hasMoreRuns = false, onLoadMoreRuns = null, isLoadingMoreRuns = false, actions = null,
}) => {
  const [query, setQuery] = useState("");
  const [focused, setFocused] = useState(false);
  const [open, setOpen] = useState(false);
  const input = useRef(null);
  const keepOpen = useRef(false);
  const search = useRunSearch(query, showChildRuns, open && focused);
  const searching = Boolean(query.trim());
  const selected = runs.find((run) => run.run_id === selectedRun) ?? null;
  const loading = searching ? search.loading : isLoadingMoreRuns;
  const hasMore = searching ? search.hasMore : hasMoreRuns;
  const options = orderRunsForSelector(searching ? search.items : runs);
  if (hasMore) options.push(MORE);
  if (runs.length === 0) return null;

  return <Box sx={{ mb: 2 }}>
    <Box sx={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 1, mb: 1 }}>
      {actions}
      <FormControlLabel sx={{ ml: "auto", mr: 0 }} label="Show child runs"
        control={<Switch size="small" checked={showChildRuns}
          onChange={(event) => onShowChildRunsChange?.(event.target.checked)} />} />
    </Box>
    <Autocomplete fullWidth size="small" value={selected} options={options} disableClearable openOnFocus
      open={open} onOpen={() => setOpen(true)} onClose={(_, reason) => {
        if (reason === "selectOption" && keepOpen.current) { keepOpen.current = false; return; }
        setOpen(false);
        if (reason === "selectOption") input.current?.blur();
      }}
      inputValue={focused ? query : selected ? formatRunLabel(selected) : ""}
      onInputChange={(_, value, reason) => { if (reason === "input") setQuery(value); }}
      filterOptions={(items) => items} isOptionEqualToValue={(left, right) => left.run_id === right.run_id}
      getOptionKey={(run) => run.loadMore ? "more" : run.run_id}
      getOptionLabel={(run) => run.loadMore ? "Load more runs" : formatRunLabel(run)}
      getOptionDisabled={(run) => run.loadMore && loading}
      onChange={(_, run) => {
        if (run?.loadMore) { keepOpen.current = true; if (searching) search.loadMore(); else onLoadMoreRuns?.(); return; }
        if (run) { onRunChange?.(run.run_id); input.current?.blur(); setFocused(false); setOpen(false); }
      }}
      loading={loading} loadingText="Searching runs…"
      noOptionsText={search.error ? search.error.message : "No matching runs"}
      renderOption={(props, run) => {
        const { key, ...optionProps } = props;
        if (run.loadMore) return <Box component="li" key={key} {...optionProps}>{search.error ? "Retry loading more runs" : loading ? "Loading…" : "Load more runs"}</Box>;
        const parentName = run.parent_run_name || runs.find((parent) => parent.run_id === run.parent_run_id)?.run_name;
        return <Box component="li" key={key} {...optionProps}
          aria-label={`${formatRunLabel(run)} · #${run.run_id} · ${formatRunSecondaryLabel(run)}`}
          sx={{ "&.MuiAutocomplete-option": { pl: run.parent_run_id != null ? 4 : 2 } }}>
          <Box sx={{ minWidth: 0, py: 0.25, overflowWrap: "anywhere" }}>
            <Box sx={{ display: "flex", alignItems: "center", flexWrap: "wrap", gap: 1 }}>
              <Typography component="span" variant="body2" fontWeight={500}>{formatRunLabel(run)}</Typography>
              <Typography component="span" variant="caption" color="text.secondary">#{run.run_id}</Typography>
              {run.parent_run_id != null && <Chip size="small" variant="outlined" color="primary"
                label={`Parent: ${parentName ? `${parentName} ` : ""}#${run.parent_run_id}`} />}
            </Box>
            <Typography variant="caption" color="text.secondary">{formatRunSecondaryLabel(run)}</Typography>
          </Box>
        </Box>;
      }}
      renderInput={(params) => <TextField {...params} inputRef={input} placeholder="Search runs…"
        onFocus={() => setFocused(true)} onBlur={() => { setFocused(false); setOpen(false); }}
        inputProps={{ ...params.inputProps, "aria-label": "Select run" }}
        InputProps={{ ...params.InputProps, endAdornment: <>
          {!focused && selected && <StatusChip status={selected.lifecycle_state} />}
          <Box component="span" sx={{ width: 24, display: "inline-flex", justifyContent: "center", flexShrink: 0 }}>
            {loading && <CircularProgress size={16} aria-label="Loading runs" />}
          </Box>
          {params.InputProps.endAdornment}
        </> }} />} />
  </Box>;
};

export default RunSelector;
