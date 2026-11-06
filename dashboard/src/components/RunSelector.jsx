import { Box, Button, FormControl, FormControlLabel, InputLabel, MenuItem, Select, Switch, Typography } from "@mui/material";
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
}) => {
  if (runs.length === 0) return null;
  const orderedRuns = orderRunsForSelector(runs);
  const selected = runs.find((run) => run.run_id === selectedRun);

  return (
    <Box sx={{ mb: 2 }}>
      <Box sx={{ display: "flex", justifyContent: "flex-end", mb: 1 }}>
        <FormControlLabel
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
      <FormControl fullWidth variant="outlined">
        <InputLabel id="run-selector-label">Select run</InputLabel>
        <Select
          labelId="run-selector-label"
          value={selectedRun || ""}
          onChange={(e) => onRunChange(Number(e.target.value))}
          label="Select run"
          renderValue={() => selected && (
            <Box title={`${formatRunLabel(selected)} · ${formatRunSecondaryLabel(selected)}`}
              sx={{ display: "flex", alignItems: "center", gap: 1, minWidth: 0 }}>
              <Typography component="span" noWrap sx={{ fontSize: "0.875rem", fontWeight: 500 }}>
                {formatRunLabel(selected)}
              </Typography>
              <StatusChip status={selected.lifecycle_state} />
            </Box>
          )}
          MenuProps={{ PaperProps: { sx: { maxWidth: "calc(100vw - 32px)" } } }}
        >
          {orderedRuns.map((run) => (
            <MenuItem key={run.run_id} value={run.run_id} aria-label={`${formatRunLabel(run)} · ${formatRunSecondaryLabel(run)}`}>
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
        <Box sx={{ display: "flex", justifyContent: "flex-end", mt: 1 }}>
          <Button size="small" onClick={onLoadMoreRuns} disabled={isLoadingMoreRuns}>
            {isLoadingMoreRuns ? "Loading" : "Load more runs"}
          </Button>
        </Box>
      )}
    </Box>
  );
};

export default RunSelector;
