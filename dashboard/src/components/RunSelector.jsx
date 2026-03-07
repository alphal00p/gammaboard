import { Box, FormControl, InputLabel, Select, MenuItem, Typography } from "@mui/material";
import { formatRunLabel, formatRunSecondaryLabel } from "../utils/runs";

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
