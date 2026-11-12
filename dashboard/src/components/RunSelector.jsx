import { Box, Button, FormControl, FormControlLabel, MenuItem, Select, Switch, Typography } from "@mui/material";
import { formatRunLabel, formatRunSecondaryLabel, orderRunsForSelector } from "../utils/runs";
import StatusChip from "./common/StatusChip";

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
      <FormControl fullWidth variant="outlined" size="small" sx={{ gridArea: "selector", minWidth: 0 }}>
        <Select
          inputProps={{ "aria-label": "Select run" }}
          value={selectedRun || ""}
          onChange={(e) => onRunChange(Number(e.target.value))}
          renderValue={() => selected && (
            <Box title={`${formatRunLabel(selected)} · ${formatRunSecondaryLabel(selected)}`}
              sx={{ display: "flex", alignItems: "center", gap: 1, minWidth: 0, pl: selected.parent_run_id != null ? 2 : 0 }}>
              <Typography component="span" noWrap sx={{ fontSize: "0.875rem", fontWeight: 500 }}>
                {formatRunLabel(selected)}
              </Typography>
              <StatusChip status={selected.lifecycle_state} />
            </Box>
          )}
          MenuProps={{ PaperProps: { sx: { maxWidth: "calc(100vw - 32px)" } } }}
        >
          {orderedRuns.map((run) => (
            <MenuItem key={run.run_id} value={run.run_id} sx={{ pl: run.parent_run_id != null ? 4 : 2 }} aria-label={`${formatRunLabel(run)} · ${formatRunSecondaryLabel(run)}`}>
              <Box sx={{ minWidth: 0, py: 0.25, whiteSpace: "normal", overflowWrap: "anywhere" }}>
                <Typography component="span" sx={{ fontSize: "0.875rem", fontWeight: 500 }}>
                  {formatRunLabel(run)}
                </Typography>
                <Typography variant="caption" component="span" display="block" color="text.secondary">
                  {formatRunSecondaryLabel(run)}
                </Typography>
              </Box>
            </MenuItem>
          ))}
        </Select>
      </FormControl>
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
