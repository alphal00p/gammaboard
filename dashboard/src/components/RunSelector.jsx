import { useState } from "react";
import { Autocomplete, Box, Button, createFilterOptions, FormControlLabel, Switch, TextField, Typography } from "@mui/material";
import { formatRunLabel, formatRunSecondaryLabel, orderRunsForSelector } from "../utils/runs";
import StatusChip from "./common/StatusChip";

const filterRuns = createFilterOptions({ stringify: (run) =>
  `${formatRunLabel(run)} #${run.run_id} ${run.lifecycle_state || ""} ${run.parent_run_id != null ? `child of #${run.parent_run_id}` : ""}` });

const RunSelector = ({
  runs,
  selectedRun,
  onRunChange,
  showChildRuns = false,
  onShowChildRunsChange = null,
  hasMoreRuns = false,
  onLoadMoreRuns = null,
  isLoadingMoreRuns = false,
  actions = null,
}) => {
  const [query, setQuery] = useState(null);
  if (runs.length === 0) return null;
  const orderedRuns = orderRunsForSelector(runs);
  const selected = runs.find((run) => run.run_id === selectedRun);

  return (
    <Box sx={{ mb: 1.5, display: "grid", alignItems: "center", gap: 1,
      gridTemplateColumns: { xs: "minmax(0, 1fr)", md: "minmax(0, 1fr) auto" },
      gridTemplateAreas: { xs: '"actions" "selector" "more"', md: '"selector actions" "more more"' } }}>
      <Box sx={{ gridArea: "actions", display: "flex", alignItems: "center", flexWrap: "wrap", gap: 1 }}>
        {actions}
        <FormControlLabel
          sx={{ ml: "auto", mr: 0 }}
          control={
            <Switch
              size="small"
              checked={showChildRuns}
              onChange={(event) => onShowChildRunsChange?.(event.target.checked)}
            />
          }
          label="Show child runs"
        />
      </Box>
      <Autocomplete size="small" disableClearable openOnFocus blurOnSelect clearOnBlur={false}
        sx={{ gridArea: "selector", minWidth: 0 }} options={orderedRuns} value={selected || null}
        inputValue={query ?? (selected ? formatRunLabel(selected) : "")}
        getOptionLabel={formatRunLabel} getOptionKey={(run) => run.run_id}
        isOptionEqualToValue={(left, right) => left.run_id === right.run_id}
        filterOptions={(options, state) => filterRuns(options, { ...state, inputValue: query ?? "" })}
        noOptionsText={hasMoreRuns ? "No matching loaded runs. Load more runs to search older entries." : "No matching runs."}
        onInputChange={(_, value, reason) => {
          // Polling replaces option objects; only typing changes the search.
          if (reason === "input" || reason === "clear") setQuery(value);
        }}
        onBlur={() => setQuery(null)}
        onChange={(_, run) => { setQuery(null); if (run) onRunChange?.(run.run_id); }}
        renderInput={(params) => <TextField {...params} placeholder="Search runs by name, ID or status"
          inputProps={{ ...params.inputProps, "aria-label": "Select run" }}
          InputProps={{ ...params.InputProps, endAdornment: <>
            {query == null && selected && <StatusChip status={selected.lifecycle_state} sx={{ mr: 1, flexShrink: 0 }} />}
            {params.InputProps.endAdornment}
          </> }} />}
        renderOption={(props, run) => {
          const { key, ...rest } = props;
          return <Box component="li" key={key} {...rest}
            sx={{ pl: run.parent_run_id != null ? "32px !important" : "16px !important" }}
            aria-label={`${formatRunLabel(run)} · #${run.run_id} · ${formatRunSecondaryLabel(run)}`}>
            <Box sx={{ minWidth: 0, py: 0.25, whiteSpace: "normal", overflowWrap: "anywhere" }}>
              <Typography variant="body2" sx={{ fontWeight: 500 }}>{formatRunLabel(run)}</Typography>
              <Typography variant="caption" component="span" display="block" color="text.secondary">
                #{run.run_id} · {formatRunSecondaryLabel(run)}
              </Typography>
            </Box>
          </Box>;
        }} />
      {hasMoreRuns && (
        <Box sx={{ gridArea: "more", display: "flex", justifyContent: "flex-end" }}>
          <Button size="small" onClick={onLoadMoreRuns} disabled={isLoadingMoreRuns}>
            {isLoadingMoreRuns ? "Loading" : "Load more runs"}
          </Button>
        </Box>
      )}
    </Box>
  );
};

export default RunSelector;
