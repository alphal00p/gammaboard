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
import { DataGrid, useGridApiRef } from "@mui/x-data-grid";

const MAX_LINES = 2000;

const levelTone = (level) => {
  switch ((level || "").toLowerCase()) {
    case "error":
      return "#ff8a80";
    case "warn":
    case "warning":
      return "#ffd180";
    case "info":
      return "#a5d6ff";
    default:
      return "#d3d7de";
  }
};

const normalizeLevel = (level) => (level || "").toLowerCase();

const formatTimestamp = (ts) => {
  if (!ts) return "-";
  const value = new Date(ts);
  if (Number.isNaN(value.getTime())) return ts;
  return value.toLocaleString();
};

const compareLogsAsc = (a, b) => {
  const aTime = Date.parse(a.ts || "");
  const bTime = Date.parse(b.ts || "");
  const aValid = Number.isFinite(aTime);
  const bValid = Number.isFinite(bTime);

  if (aValid && bValid && aTime !== bTime) return aTime - bTime;
  if (aValid !== bValid) return aValid ? -1 : 1;
  const aId = Number(a.id);
  const bId = Number(b.id);
  if (Number.isFinite(aId) && Number.isFinite(bId) && aId !== bId) return aId - bId;
  return String(a.id).localeCompare(String(b.id));
};

const mergeLogs = (previous, incoming) => {
  const merged = new Map(previous.map((entry) => [entry.id, entry]));
  for (const entry of incoming) {
    if (!entry || entry.id == null) continue;
    merged.set(entry.id, entry);
  }

  const out = Array.from(merged.values()).sort(compareLogsAsc);
  if (out.length <= MAX_LINES) return out;
  return out.slice(out.length - MAX_LINES);
};

const WorkerLogsPanel = ({ logs, runId }) => {
  const apiRef = useGridApiRef();
  const sourceLogs = Array.isArray(logs) ? logs : [];

  const [bufferedLogs, setBufferedLogs] = useState([]);
  const [displayedLogs, setDisplayedLogs] = useState([]);
  const [selectedLogId, setSelectedLogId] = useState(null);
  const [tailEnabled, setTailEnabled] = useState(true);
  const [paused, setPaused] = useState(false);
  const [levelFilter, setLevelFilter] = useState([]);
  const [roleFilter, setRoleFilter] = useState("all");
  const [workerFilter, setWorkerFilter] = useState("all");
  const [search, setSearch] = useState("");

  useEffect(() => {
    setBufferedLogs([]);
    setDisplayedLogs([]);
    setSelectedLogId(null);
    setTailEnabled(true);
    setPaused(false);
    setLevelFilter([]);
    setRoleFilter("all");
    setWorkerFilter("all");
    setSearch("");
  }, [runId]);

  useEffect(() => {
    setBufferedLogs((prev) => mergeLogs(prev, sourceLogs));
  }, [sourceLogs]);

  useEffect(() => {
    if (!paused) setDisplayedLogs(bufferedLogs);
  }, [bufferedLogs, paused]);

  const workerOptions = useMemo(() => {
    const values = new Set(displayedLogs.map((entry) => entry.worker_id).filter(Boolean));
    return Array.from(values).sort();
  }, [displayedLogs]);

  const roleOptions = useMemo(() => {
    const values = new Set(displayedLogs.map((entry) => entry.role).filter(Boolean));
    return Array.from(values).sort();
  }, [displayedLogs]);

  const filteredRows = useMemo(() => {
    const text = search.trim().toLowerCase();

    return displayedLogs.filter((entry) => {
      const level = normalizeLevel(entry.level);
      if (levelFilter.length > 0 && !levelFilter.includes(level)) return false;
      if (roleFilter !== "all" && entry.role !== roleFilter) return false;
      if (workerFilter !== "all" && entry.worker_id !== workerFilter) return false;

      if (text) {
        const payload = `${entry.message || ""} ${JSON.stringify(entry.fields || {})}`.toLowerCase();
        if (!payload.includes(text)) return false;
      }

      return true;
    });
  }, [displayedLogs, levelFilter, roleFilter, workerFilter, search]);

  useEffect(() => {
    if (!tailEnabled || paused || filteredRows.length === 0) return;
    const rowIndex = filteredRows.length - 1;
    const timeout = setTimeout(() => {
      apiRef.current?.scrollToIndexes({ rowIndex });
    }, 0);
    return () => clearTimeout(timeout);
  }, [apiRef, filteredRows.length, paused, tailEnabled]);

  const selectedLog = useMemo(
    () => filteredRows.find((entry) => entry.id === selectedLogId) || null,
    [filteredRows, selectedLogId],
  );

  const backlogCount = Math.max(bufferedLogs.length - displayedLogs.length, 0);
  const levelOptions = ["error", "warn", "info", "debug", "trace"];

  const columns = useMemo(
    () => [
      {
        field: "ts",
        headerName: "Timestamp",
        width: 220,
        renderCell: (params) => formatTimestamp(params.value),
      },
      {
        field: "level",
        headerName: "Level",
        width: 90,
        renderCell: (params) => (
          <Box component="span" sx={{ color: levelTone(params.value), fontWeight: 700 }}>
            {(params.value || "unknown").toUpperCase()}
          </Box>
        ),
      },
      { field: "role", headerName: "Role", width: 180 },
      { field: "worker_id", headerName: "Worker", width: 220 },
      {
        field: "message",
        headerName: "Message",
        flex: 1,
        minWidth: 300,
        renderCell: (params) => (
          <Box
            component="span"
            sx={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              display: "block",
              width: "100%",
            }}
          >
            {params.value || ""}
          </Box>
        ),
      },
    ],
    [],
  );

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Worker Logs
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

        <FormControl size="small" sx={{ minWidth: 180 }}>
          <InputLabel id="log-role-label">Role</InputLabel>
          <Select
            labelId="log-role-label"
            value={roleFilter}
            label="Role"
            onChange={(event) => setRoleFilter(event.target.value)}
          >
            <MenuItem value="all">All Roles</MenuItem>
            {roleOptions.map((role) => (
              <MenuItem key={role} value={role}>
                {role}
              </MenuItem>
            ))}
          </Select>
        </FormControl>

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
        <ToggleButton size="small" value="pause" selected={paused} onChange={() => setPaused((value) => !value)}>
          Pause
        </ToggleButton>

        <Button
          size="small"
          variant="outlined"
          onClick={() => {
            setBufferedLogs([]);
            setDisplayedLogs([]);
            setSelectedLogId(null);
          }}
        >
          Clear
        </Button>
      </Stack>

      {paused && backlogCount > 0 ? (
        <Alert severity="info" sx={{ mb: 1.5 }}>
          Paused. {backlogCount} new log lines buffered.
        </Alert>
      ) : null}

      <Paper
        variant="outlined"
        sx={{
          height: { xs: 320, md: 460 },
          mb: 1.5,
          bgcolor: "#101317",
          color: "#d3d7de",
        }}
      >
        <DataGrid
          apiRef={apiRef}
          rows={filteredRows}
          columns={columns}
          getRowId={(row) => row.id}
          density="compact"
          rowHeight={30}
          disableColumnMenu
          disableRowSelectionOnClick={false}
          hideFooter
          onRowClick={(params) => setSelectedLogId(params.id)}
          sx={{
            border: 0,
            bgcolor: "transparent",
            color: "inherit",
            "& .MuiDataGrid-columnHeaders": {
              bgcolor: "#171c22",
              borderBottom: "1px solid #2a3340",
            },
            "& .MuiDataGrid-columnHeaderTitle": {
              fontWeight: 700,
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
            },
            "& .MuiDataGrid-cell": {
              borderBottom: "1px solid #1f2630",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
              fontSize: "0.78rem",
            },
            "& .MuiDataGrid-row.Mui-selected": {
              bgcolor: "#233040",
            },
            "& .MuiDataGrid-row:hover": {
              bgcolor: "#1a2533",
            },
          }}
        />
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
                level: selectedLog.level,
                role: selectedLog.role,
                worker_id: selectedLog.worker_id,
                message: selectedLog.message,
                event_type: selectedLog.event_type,
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
    </Box>
  );
};

export default WorkerLogsPanel;
