import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Box,
  Button,
  FormControl,
  InputLabel,
  MenuItem,
  OutlinedInput,
  Paper,
  Select,
  Stack,
  TextField,
  ToggleButton,
  Typography,
} from "@mui/material";
import { formatDateTime } from "../utils/formatters";
import { buildLogSearchText } from "../utils/logs";
import { formatRunLabel } from "../utils/runs";

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

const normalizeLevel = (level) => {
  const normalized = (level || "").toLowerCase();
  return normalized === "warning" ? "warn" : normalized;
};
const EMPTY_LEVEL_FILTER = [];

const toDecimalId = (value) => {
  if (value == null) return null;
  const normalized = String(value).trim();
  if (!/^\d+$/.test(normalized)) return null;
  return normalized.replace(/^0+(?=\d)/, "");
};

const isLogIdAfter = (entryId, thresholdId) => {
  const entry = toDecimalId(entryId);
  const threshold = toDecimalId(thresholdId);
  if (entry == null || threshold == null) return false;
  if (entry.length !== threshold.length) return entry.length > threshold.length;
  return entry > threshold;
};

const WorkerLogsPanel = ({
  logs,
  runId,
  runs = [],
  title = "Worker Logs",
  variant = "full",
  defaultLevelFilter = EMPTY_LEVEL_FILTER,
  onOpenFullLogs = null,
}) => {
  const compact = variant === "compact";
  const sourceLogs = useMemo(
    () =>
      (Array.isArray(logs) ? logs : []).map((entry) =>
        entry?._searchText ? entry : { ...entry, _searchText: buildLogSearchText(entry) },
      ),
    [logs],
  );

  const [pausedSnapshot, setPausedSnapshot] = useState(null);
  const [selectedLogId, setSelectedLogId] = useState(null);
  const [tailEnabled, setTailEnabled] = useState(true);
  const [paused, setPaused] = useState(false);
  const [levelFilter, setLevelFilter] = useState(defaultLevelFilter);
  const [runFilter, setRunFilter] = useState(runId ?? "all");
  const [workerFilter, setWorkerFilter] = useState("all");
  const [search, setSearch] = useState("");
  const [clearAfterId, setClearAfterId] = useState(null);

  useEffect(() => {
    setPausedSnapshot(null);
    setSelectedLogId(null);
    setTailEnabled(true);
    setPaused(false);
    setLevelFilter(defaultLevelFilter);
    setWorkerFilter("all");
    setSearch("");
    setClearAfterId(null);
  }, [defaultLevelFilter, runId]);

  const visibleLiveLogs = useMemo(() => {
    if (clearAfterId == null) return sourceLogs;
    return sourceLogs.filter((entry) => entry?.id != null && isLogIdAfter(entry.id, clearAfterId));
  }, [sourceLogs, clearAfterId]);

  const displayedLogs = useMemo(() => {
    if (!paused) return visibleLiveLogs;
    return pausedSnapshot ?? visibleLiveLogs;
  }, [visibleLiveLogs, paused, pausedSnapshot]);

  const workerOptions = useMemo(() => {
    const values = new Set(displayedLogs.map((entry) => entry.worker_id).filter(Boolean));
    return Array.from(values).sort();
  }, [displayedLogs]);

  const runOptions = useMemo(() => {
    const list = Array.isArray(runs) ? runs : [];
    return list
      .map((run) => run.run_id)
      .filter((id) => Number.isFinite(Number(id)))
      .sort((a, b) => a - b);
  }, [runs]);

  const runLabel = (id) => {
    const numericId = Number(id);
    const match = runs.find((run) => run.run_id === numericId);
    if (!match) return `Run #${numericId}`;
    return formatRunLabel(match);
  };

  useEffect(() => {
    if (runId == null) return;
    setRunFilter((current) => {
      if (current === "all") return runId;
      if (!runOptions.includes(Number(current))) return runId;
      return current;
    });
  }, [runId, runOptions]);

  useEffect(() => {
    setRunFilter((current) => {
      if (current === "all") return current;
      if (!runOptions.includes(Number(current))) return "all";
      return current;
    });
  }, [runOptions]);

  const filteredRows = useMemo(() => {
    const text = search.trim().toLowerCase();

    return displayedLogs.filter((entry) => {
      const level = normalizeLevel(entry.level);
      if (levelFilter.length > 0 && !levelFilter.includes(level)) return false;
      if (runFilter !== "all" && Number(entry.run_id) !== Number(runFilter)) return false;
      if (workerFilter !== "all" && entry.worker_id !== workerFilter) return false;

      if (text) {
        const payload = entry._searchText || "";
        if (!payload.includes(text)) return false;
      }

      return true;
    });
  }, [displayedLogs, levelFilter, runFilter, workerFilter, search]);

  const selectedLog = useMemo(
    () => filteredRows.find((entry) => entry.id === selectedLogId) || null,
    [filteredRows, selectedLogId],
  );

  const pausedSize = pausedSnapshot?.length ?? 0;
  const backlogCount = paused ? Math.max(visibleLiveLogs.length - pausedSize, 0) : 0;
  const levelOptions = ["error", "warn", "info", "debug", "trace"];

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        {title}
      </Typography>

      <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" sx={{ mb: 1.5 }}>
        <FormControl size="small" sx={{ minWidth: 180 }}>
          <InputLabel id="log-level-label">Level</InputLabel>
          <Select
            labelId="log-level-label"
            multiple
            value={levelFilter}
            onChange={(event) => {
              const next = event.target.value;
              setLevelFilter(typeof next === "string" ? next.split(",") : next);
            }}
            input={<OutlinedInput label="Level" />}
          >
            {levelOptions.map((level) => (
              <MenuItem key={level} value={level}>
                {level.toUpperCase()}
              </MenuItem>
            ))}
          </Select>
        </FormControl>

        {!compact && (
          <FormControl size="small" sx={{ minWidth: 220 }}>
            <InputLabel id="log-run-label">Run</InputLabel>
            <Select
              labelId="log-run-label"
              value={runFilter}
              label="Run"
              onChange={(event) => setRunFilter(event.target.value)}
            >
              <MenuItem value="all">All Runs</MenuItem>
              {runOptions.map((run) => (
                <MenuItem key={run} value={run}>
                  {runLabel(run)}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
        )}

        {!compact && (
          <FormControl size="small" sx={{ minWidth: 200 }}>
            <InputLabel id="log-worker-label">Worker</InputLabel>
            <Select
              labelId="log-worker-label"
              value={workerFilter}
              label="Worker"
              onChange={(event) => setWorkerFilter(event.target.value)}
            >
              <MenuItem value="all">All Workers</MenuItem>
              {workerOptions.map((workerId) => (
                <MenuItem key={workerId} value={workerId}>
                  {workerId}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
        )}

        <TextField
          size="small"
          label="Search"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          sx={{ minWidth: 260, flexGrow: 1 }}
        />

        <ToggleButton
          size="small"
          value="tail"
          selected={tailEnabled}
          onChange={() => setTailEnabled((value) => !value)}
        >
          Tail
        </ToggleButton>
        <ToggleButton
          size="small"
          value="pause"
          selected={paused}
          onChange={() =>
            setPaused((value) => {
              const next = !value;
              if (next) {
                setPausedSnapshot(visibleLiveLogs);
              } else {
                setPausedSnapshot(null);
              }
              return next;
            })
          }
        >
          Pause
        </ToggleButton>

        <Button
          size="small"
          variant="outlined"
          onClick={() => {
            setPausedSnapshot(null);
            setSelectedLogId(null);
            const last = sourceLogs[sourceLogs.length - 1];
            setClearAfterId(last?.id ?? clearAfterId);
          }}
        >
          Clear
        </Button>
        {compact && typeof onOpenFullLogs === "function" && (
          <Button size="small" variant="text" onClick={onOpenFullLogs}>
            Open Full Logs
          </Button>
        )}
      </Stack>

      {paused && backlogCount > 0 ? (
        <Alert severity="info" sx={{ mb: 1.5 }}>
          Paused. {backlogCount} new log lines buffered.
        </Alert>
      ) : null}

      <Paper
        variant="outlined"
        sx={{
          height: compact ? { xs: 260, md: 320 } : { xs: 320, md: 460 },
          mb: 1.5,
          overflow: "auto",
        }}
      >
        <Box component="table" sx={{ width: "100%", borderCollapse: "collapse", tableLayout: "fixed" }}>
          <Box component="thead">
            <Box component="tr">
              <Box
                component="th"
                sx={{ textAlign: "left", p: 1, borderBottom: "1px solid", borderColor: "divider", width: 220 }}
              >
                Timestamp
              </Box>
              <Box
                component="th"
                sx={{ textAlign: "left", p: 1, borderBottom: "1px solid", borderColor: "divider", width: 90 }}
              >
                Level
              </Box>
              {!compact && (
                <Box
                  component="th"
                  sx={{ textAlign: "left", p: 1, borderBottom: "1px solid", borderColor: "divider", width: 220 }}
                >
                  Worker
                </Box>
              )}
              {!compact && (
                <Box
                  component="th"
                  sx={{ textAlign: "left", p: 1, borderBottom: "1px solid", borderColor: "divider", width: 260 }}
                >
                  Run
                </Box>
              )}
              <Box component="th" sx={{ textAlign: "left", p: 1, borderBottom: "1px solid", borderColor: "divider" }}>
                Message
              </Box>
            </Box>
          </Box>
          <Box component="tbody">
            {filteredRows.map((row, index) => (
              <Box
                component="tr"
                key={row.id != null ? String(row.id) : `row-${index}`}
                onClick={compact ? undefined : () => setSelectedLogId(row.id)}
                sx={{
                  cursor: compact ? "default" : "pointer",
                  bgcolor: !compact && selectedLogId === row.id ? "action.selected" : "transparent",
                  "&:hover": { bgcolor: compact ? "transparent" : "action.hover" },
                }}
              >
                <Box
                  component="td"
                  sx={{ p: 1, borderBottom: "1px solid", borderColor: "divider", whiteSpace: "nowrap" }}
                >
                  {formatDateTime(row.ts, "-")}
                </Box>
                <Box
                  component="td"
                  sx={{
                    p: 1,
                    borderBottom: "1px solid",
                    borderColor: "divider",
                    color: levelTone(row.level),
                    fontWeight: 700,
                  }}
                >
                  {(row.level || "unknown").toUpperCase()}
                </Box>
                {!compact && (
                  <Box
                    component="td"
                    sx={{ p: 1, borderBottom: "1px solid", borderColor: "divider", whiteSpace: "nowrap" }}
                  >
                    {row.worker_id || "-"}
                  </Box>
                )}
                {!compact && (
                  <Box
                    component="td"
                    sx={{ p: 1, borderBottom: "1px solid", borderColor: "divider", whiteSpace: "nowrap" }}
                  >
                    {row.run_id != null ? runLabel(row.run_id) : "-"}
                  </Box>
                )}
                <Box
                  component="td"
                  sx={{
                    p: 1,
                    borderBottom: "1px solid",
                    borderColor: "divider",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {row.message || ""}
                </Box>
              </Box>
            ))}
            {filteredRows.length === 0 && (
              <Box component="tr">
                <Box component="td" colSpan={compact ? 3 : 5} sx={{ p: 2, color: "text.secondary" }}>
                  No logs match the current filters.
                </Box>
              </Box>
            )}
          </Box>
        </Box>
      </Paper>

      {!compact && (
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
                  run_id: selectedLog.run_id,
                  node_id: selectedLog.node_id,
                  level: selectedLog.level,
                  worker_id: selectedLog.worker_id,
                  message: selectedLog.message,
                  fields: selectedLog.fields || {},
                },
                null,
                2,
              )}
            </Box>
          ) : (
            <Typography variant="body2" color="text.secondary">
              Click a log line to inspect raw fields.
            </Typography>
          )}
        </Paper>
      )}
    </Box>
  );
};

export default WorkerLogsPanel;
