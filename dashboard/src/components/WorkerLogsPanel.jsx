import { useEffect, useMemo, useState } from "react";
import { Alert, Box, Button, Paper, Stack, Typography } from "@mui/material";
import { DataGrid, GridToolbar } from "@mui/x-data-grid";
import { formatDateTime } from "../utils/formatters";
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

const runLabel = (runs, id) => {
  const numericId = Number(id);
  const match = runs.find((run) => run.run_id === numericId);
  if (!match) return `Run #${numericId}`;
  return formatRunLabel(match);
};

const WorkerLogsPanel = ({ logs, runs = [], title = "Worker Logs", defaultLevelFilter = [] }) => {
  const defaultLevelFilterKey = (Array.isArray(defaultLevelFilter) ? defaultLevelFilter : []).join("|");
  const [pausedSnapshot, setPausedSnapshot] = useState(null);
  const [selectedLogId, setSelectedLogId] = useState(null);
  const [paused, setPaused] = useState(false);
  const [clearAfterId, setClearAfterId] = useState(null);
  const [filterModel, setFilterModel] = useState({
    items: [],
    quickFilterValues: [],
  });

  useEffect(() => {
    const levelItems =
      defaultLevelFilterKey.length === 0
        ? []
        : defaultLevelFilterKey.split("|").map((level, index) => ({
            id: `level-${index}`,
            field: "level",
            operator: "is",
            value: level,
          }));
    setPausedSnapshot(null);
    setSelectedLogId(null);
    setPaused(false);
    setClearAfterId(null);
    setFilterModel({
      items: levelItems,
      quickFilterValues: [],
    });
  }, [defaultLevelFilterKey]);

  const visibleLiveLogs = useMemo(() => {
    const list = Array.isArray(logs) ? logs : [];
    if (clearAfterId == null) return list;
    return list.filter((entry) => entry?.id != null && isLogIdAfter(entry.id, clearAfterId));
  }, [logs, clearAfterId]);

  const displayedLogs = useMemo(() => {
    if (!paused) return visibleLiveLogs;
    return pausedSnapshot ?? visibleLiveLogs;
  }, [paused, pausedSnapshot, visibleLiveLogs]);

  const rows = useMemo(
    () =>
      displayedLogs.map((entry) => ({
        ...entry,
        id: String(entry.id),
        run_label: entry.run_id != null ? runLabel(runs, entry.run_id) : "-",
        worker_label: entry.worker_id || "-",
        ts_label: formatDateTime(entry.ts, "-"),
        ts_ms: entry.ts ? Date.parse(entry.ts) : null,
      })),
    [displayedLogs, runs],
  );

  const selectedLog = useMemo(() => rows.find((row) => row.id === selectedLogId) || null, [rows, selectedLogId]);

  const runValueOptions = useMemo(
    () =>
      runs
        .map((run) => ({
          value: formatRunLabel(run),
          label: formatRunLabel(run),
        }))
        .sort((a, b) => a.label.localeCompare(b.label)),
    [runs],
  );

  const workerValueOptions = useMemo(() => {
    const values = new Set(rows.map((row) => row.worker_label).filter((value) => value && value !== "-"));
    return Array.from(values)
      .sort()
      .map((value) => ({ value, label: value }));
  }, [rows]);

  const columns = useMemo(
    () => [
      {
        field: "ts_ms",
        headerName: "Timestamp",
        minWidth: 220,
        flex: 0.9,
        renderCell: (params) => params.row.ts_label,
      },
      {
        field: "level",
        headerName: "Level",
        minWidth: 110,
        type: "singleSelect",
        valueOptions: ["error", "warn", "info", "debug", "trace"],
        renderCell: (params) => (
          <Box component="span" sx={{ color: levelTone(params.value), fontWeight: 700 }}>
            {String(params.value || "unknown").toUpperCase()}
          </Box>
        ),
      },
      {
        field: "worker_label",
        headerName: "Worker",
        minWidth: 220,
        flex: 0.9,
        type: "singleSelect",
        valueOptions: workerValueOptions,
      },
      {
        field: "run_label",
        headerName: "Run",
        minWidth: 260,
        flex: 1,
        type: "singleSelect",
        valueOptions: runValueOptions,
      },
      {
        field: "message",
        headerName: "Message",
        minWidth: 320,
        flex: 1.8,
      },
    ],
    [runValueOptions, workerValueOptions],
  );

  const backlogCount = paused && pausedSnapshot ? Math.max(visibleLiveLogs.length - pausedSnapshot.length, 0) : 0;

  return (
    <Box sx={{ mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        {title}
      </Typography>

      <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap" sx={{ mb: 1.5 }}>
        <Button
          size="small"
          variant={paused ? "contained" : "outlined"}
          onClick={() =>
            setPaused((value) => {
              const next = !value;
              setPausedSnapshot(next ? visibleLiveLogs : null);
              return next;
            })
          }
        >
          {paused ? "Resume" : "Pause"}
        </Button>

        <Button
          size="small"
          variant="outlined"
          onClick={() => {
            setPausedSnapshot(null);
            setSelectedLogId(null);
            const last = logs[logs.length - 1];
            setClearAfterId(last?.id ?? clearAfterId);
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

      <Paper variant="outlined" sx={{ mb: 1.5 }}>
        <DataGrid
          rows={rows}
          columns={columns}
          density="compact"
          disableRowSelectionOnClick={false}
          pageSizeOptions={[25, 50, 100, 200]}
          initialState={{
            pagination: { paginationModel: { pageSize: 50, page: 0 } },
            sorting: { sortModel: [{ field: "ts", sort: "desc" }] },
          }}
          filterModel={filterModel}
          onFilterModelChange={setFilterModel}
          onRowClick={(params) => setSelectedLogId(String(params.id))}
          slots={{ toolbar: GridToolbar }}
          slotProps={{
            toolbar: {
              showQuickFilter: true,
              quickFilterProps: { debounceMs: 250 },
            },
          }}
          localeText={{
            noRowsLabel: "No logs match the current filters.",
          }}
          sx={{
            border: 0,
            minHeight: 420,
            "& .MuiDataGrid-cell:focus, & .MuiDataGrid-columnHeader:focus": { outline: "none" },
            "& .selected-log-row": { backgroundColor: "action.selected" },
          }}
          getRowClassName={(params) => (String(params.id) === selectedLogId ? "selected-log-row" : "")}
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
            Select a log row to inspect raw fields.
          </Typography>
        )}
      </Paper>
    </Box>
  );
};

export default WorkerLogsPanel;
