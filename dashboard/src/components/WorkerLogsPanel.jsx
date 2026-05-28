import { useMemo, useState } from "react";
import {
  Alert,
  Box,
  Button,
  Checkbox,
  FormControl,
  FormControlLabel,
  InputLabel,
  MenuItem,
  Paper,
  Select,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import { formatDateTime } from "../utils/formatters";
import { asArray } from "../utils/collections";
import { formatRunLabel, formatRunSecondaryLabel, orderRunsForSelector } from "../utils/runs";

const levelTone = (level) => {
  switch ((level || "").toLowerCase()) {
    case "error":
      return "error.main";
    case "warn":
    case "warning":
      return "warning.main";
    case "info":
      return "info.main";
    default:
      return "text.secondary";
  }
};

const WorkerLogsPanel = ({
  items,
  filters,
  setFilters,
  workerOptions,
  hasMoreOlder,
  isLoading,
  error,
  refresh,
  loadOlder,
  title = "Node Logs",
  runs = [],
}) => {
  const [selectedLogId, setSelectedLogId] = useState(null);
  const logItems = asArray(items);
  const runOptions = useMemo(() => orderRunsForSelector(asArray(runs)), [runs]);
  const selectedLog = useMemo(
    () => logItems.find((entry) => String(entry.id) === selectedLogId) || null,
    [logItems, selectedLogId],
  );

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        {title}
      </Typography>

      <Stack direction="row" spacing={1.5} useFlexGap flexWrap="wrap" sx={{ mb: 1.5 }}>
        <FormControl size="small" sx={{ minWidth: 260 }}>
          <InputLabel id="worker-log-filter-run">Run</InputLabel>
          <Select
            labelId="worker-log-filter-run"
            value={filters.runId}
            label="Run"
            onChange={(event) => setFilters((current) => ({ ...current, runId: event.target.value }))}
          >
            <MenuItem value="">All runs</MenuItem>
            {runOptions.map((run) => (
              <MenuItem key={run.run_id} value={String(run.run_id)}>
                {formatRunLabel(run)} #{run.run_id} | {formatRunSecondaryLabel(run)}
              </MenuItem>
            ))}
          </Select>
        </FormControl>

        <FormControlLabel
          control={
            <Checkbox
              checked={filters.includeChildren === true}
              onChange={(event) =>
                setFilters((current) => ({ ...current, includeChildren: event.target.checked }))
              }
              disabled={!filters.runId}
            />
          }
          label="Include children"
          sx={{ mr: 0.5 }}
        />

        <FormControl size="small" sx={{ minWidth: 150 }}>
          <InputLabel id="worker-log-filter-source">Source</InputLabel>
          <Select
            labelId="worker-log-filter-source"
            value={filters.source}
            label="Source"
            onChange={(event) => setFilters((current) => ({ ...current, source: event.target.value }))}
          >
            <MenuItem value="">All sources</MenuItem>
            <MenuItem value="worker">worker</MenuItem>
            <MenuItem value="server">server</MenuItem>
            <MenuItem value="control">control</MenuItem>
          </Select>
        </FormControl>

        <FormControl size="small" sx={{ minWidth: 220 }}>
          <InputLabel id="worker-log-filter-worker">Node</InputLabel>
          <Select
            labelId="worker-log-filter-worker"
            value={filters.nodeName}
            label="Node"
            onChange={(event) => setFilters((current) => ({ ...current, nodeName: event.target.value }))}
          >
            <MenuItem value="">All nodes</MenuItem>
            {workerOptions.map((workerId) => (
              <MenuItem key={workerId} value={workerId}>
                {workerId}
              </MenuItem>
            ))}
          </Select>
        </FormControl>

        <FormControl size="small" sx={{ minWidth: 150 }}>
          <InputLabel id="worker-log-filter-level">Level</InputLabel>
          <Select
            labelId="worker-log-filter-level"
            value={filters.level}
            label="Level"
            onChange={(event) => setFilters((current) => ({ ...current, level: event.target.value }))}
          >
            <MenuItem value="">All levels</MenuItem>
            <MenuItem value="error">error</MenuItem>
            <MenuItem value="warn">warn</MenuItem>
            <MenuItem value="info">info</MenuItem>
            <MenuItem value="debug">debug</MenuItem>
            <MenuItem value="trace">trace</MenuItem>
          </Select>
        </FormControl>

        <TextField
          size="small"
          label="Search"
          value={filters.search}
          onChange={(event) => setFilters((current) => ({ ...current, search: event.target.value }))}
          sx={{ minWidth: 280, flexGrow: 1 }}
        />

        <Button size="small" variant="outlined" onClick={refresh} disabled={isLoading}>
          Refresh
        </Button>
        <Button size="small" variant="outlined" onClick={loadOlder} disabled={isLoading || !hasMoreOlder}>
          Load older
        </Button>
      </Stack>

      {error ? (
        <Alert severity="error" sx={{ mb: 1.5 }}>
          {error?.message || "Failed to fetch logs."}
        </Alert>
      ) : null}

      <Paper variant="outlined" sx={{ mb: 1.5 }}>
        <TableContainer sx={{ maxHeight: 520 }}>
          <Table stickyHeader size="small">
            <TableHead>
              <TableRow>
                <TableCell sx={{ width: 220 }}>Timestamp</TableCell>
                <TableCell sx={{ width: 120 }}>Source</TableCell>
                <TableCell sx={{ width: 110 }}>Level</TableCell>
                <TableCell sx={{ width: 110 }}>Run</TableCell>
                <TableCell sx={{ width: 220 }}>Node</TableCell>
                <TableCell>Message</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {logItems.map((entry) => (
                <TableRow
                  key={entry.id}
                  hover
                  selected={String(entry.id) === selectedLogId}
                  onClick={() => setSelectedLogId(String(entry.id))}
                  sx={{ cursor: "pointer" }}
                >
                  <TableCell>{formatDateTime(entry.ts, "-")}</TableCell>
                  <TableCell>{entry.source || "-"}</TableCell>
                  <TableCell>
                    <Box component="span" sx={{ color: levelTone(entry.level), fontWeight: 700 }}>
                      {String(entry.level || "unknown").toUpperCase()}
                    </Box>
                  </TableCell>
                  <TableCell>{entry.run_id != null ? String(entry.run_id) : "-"}</TableCell>
                  <TableCell>{entry.node_name || entry.node_uuid || "-"}</TableCell>
                  <TableCell sx={{ fontFamily: "monospace" }}>{entry.message || ""}</TableCell>
                </TableRow>
              ))}
              {logItems.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6}>
                    <Typography variant="body2" color="text.secondary">
                      No logs match the current filters.
                    </Typography>
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </TableContainer>
      </Paper>

      <Paper variant="outlined" sx={{ p: 1.5 }}>
        <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
          Selected Log Details
        </Typography>
        {selectedLog ? (
          <Box
            component="pre"
            sx={{
              m: 0,
              fontSize: "0.76rem",
              overflowX: "auto",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
            }}
          >
            {JSON.stringify(
              {
                id: selectedLog.id,
                ts: selectedLog.ts,
                source: selectedLog.source,
                run_id: selectedLog.run_id,
                node_name: selectedLog.node_name,
                node_uuid: selectedLog.node_uuid,
                level: selectedLog.level,
                target: selectedLog.target,
                message: selectedLog.message,
                fields: selectedLog.fields || {},
              },
              null,
              2,
            )}
          </Box>
        ) : (
          <Typography variant="body2" color="text.secondary">
            Select a log row to inspect raw fields.
          </Typography>
        )}
      </Paper>
    </Box>
  );
};

export default WorkerLogsPanel;
