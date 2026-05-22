import { Box, FormControl, FormControlLabel, InputLabel, MenuItem, Select, Switch, Typography } from "@mui/material";
import { formatRunLabel, formatRunSecondaryLabel, orderRunsForSelector } from "../utils/runs";

const RunSelector = ({ runs, selectedRun, onRunChange, showChildRuns = false, onShowChildRunsChange = null }) => {
  if (runs.length === 0) return null;
  const orderedRuns = orderRunsForSelector(runs);

  return (
    <Box sx={{ mb: 3 }}>
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
        <InputLabel id="run-selector-label">Select Run</InputLabel>
        <Select
          labelId="run-selector-label"
          value={selectedRun || ""}
          onChange={(e) => onRunChange(Number(e.target.value))}
          label="Select Run"
        >
          {orderedRuns.map((run) => (
            <MenuItem key={run.run_id} value={run.run_id}>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Typography component="span" sx={{ fontWeight: 500 }}>
                  {formatRunLabel(run)}
                </Typography>
                <Typography component="span" color="text.secondary">
                  {formatRunSecondaryLabel(run)}
                </Typography>
              </Box>
            </MenuItem>
          ))}
        </Select>
      </FormControl>
    </Box>
  );
};

export default RunSelector;
