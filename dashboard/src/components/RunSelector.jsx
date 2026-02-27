import { Box, FormControl, InputLabel, Select, MenuItem, Typography } from "@mui/material";

const RunSelector = ({ runs, selectedRun, onRunChange }) => {
  if (runs.length === 0) return null;

  return (
    <Box sx={{ mb: 3 }}>
      <FormControl fullWidth variant="outlined">
        <InputLabel id="run-selector-label">Select Run</InputLabel>
        <Select
          labelId="run-selector-label"
          value={selectedRun || ""}
          onChange={(e) => onRunChange(Number(e.target.value))}
          label="Select Run"
        >
          {runs.map((run) => (
            <MenuItem key={run.run_id} value={run.run_id}>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Typography component="span" sx={{ fontWeight: 500 }}>
                  {run.run_name ? `${run.run_name} (#${run.run_id})` : `Run #${run.run_id}`}
                </Typography>
                <Typography component="span" color="text.secondary">
                  • {run.run_status} • processed {(run.batches_completed || 0).toLocaleString()} • queued now{" "}
                  {(run.total_batches || 0).toLocaleString()}
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
