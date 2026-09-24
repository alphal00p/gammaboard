import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  FormControlLabel,
  Stack,
  Table as MuiTable,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TableSortLabel,
  Switch,
  Tooltip,
  Typography,
} from "@mui/material";
import { useRef, useState } from "react";
import { apiUrl } from "../../services/api";
import { asArray } from "../../utils/collections";
import { formatCentralValueWithError, formatScientific } from "../../utils/formatters";
import { renderStructuredValue } from "./BasicPanels";
import SamplePointValue from "./SamplePointValue";
import { downloadTextFile } from "./FigureExportActions";
import { readHistogramBundleSelectedValue, writeHistogramBundlePanelValue } from "./histogramUtils";

const requestHistogramBundleExport = async (payload, format) => {
  const response = await fetch(apiUrl("/histogram-bundle/export"), {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ payload, format }),
  });
  if (!response.ok) {
    let message = `HTTP ${response.status}`;
    try {
      const body = await response.json();
      if (typeof body?.error === "string" && body.error.trim()) message = body.error.trim();
    } catch {
      // Keep fallback message.
    }
    throw new Error(message);
  }
  return response.json();
};

const rowToneStyle = (rowTone) => {
  if (rowTone === "success") {
    return {
      bgcolor: "rgba(20, 184, 166, 0.08)",
      "&:hover": { bgcolor: "rgba(20, 184, 166, 0.14)" },
    };
  }
  if (rowTone === "min") {
    return {
      bgcolor: "rgba(37, 99, 235, 0.07)",
      "&:hover": { bgcolor: "rgba(37, 99, 235, 0.12)" },
    };
  }
  if (rowTone === "max") {
    return {
      bgcolor: "rgba(234, 88, 12, 0.07)",
      "&:hover": { bgcolor: "rgba(234, 88, 12, 0.12)" },
    };
  }
  if (rowTone === "min_max") {
    return {
      bgcolor: "rgba(107, 114, 128, 0.08)",
      "&:hover": { bgcolor: "rgba(107, 114, 128, 0.14)" },
    };
  }
  return {};
};

const rowToneChipColor = (rowTone) => {
  if (rowTone === "success") return "success";
  if (rowTone === "min") return "info";
  if (rowTone === "max") return "warning";
  return "default";
};

const compareTableValues = (left, right, direction) => {
  // Missing results remain last in either direction; compare unformatted values.
  if (left == null) return right == null ? 0 : 1;
  if (right == null) return -1;
  const a = left === "∞" ? Infinity : left;
  const b = right === "∞" ? Infinity : right;
  const order = typeof a === "number" && typeof b === "number"
    ? (a > b ? 1 : a < b ? -1 : 0)
    : String(a).localeCompare(String(b), undefined, { numeric: true });
  return direction === "desc" ? -order : order;
};

const BundleUploadControls = ({ state, uploadedBundles, bundleUploadError, onUploadBundle, onRemoveBundle, inputRef }) => (
  <Box sx={{ mb: 1.5 }}>
    <input
      ref={inputRef}
      type="file"
      accept="application/json,.json"
      style={{ display: "none" }}
      onChange={(event) => onUploadBundle?.(state?.panel_id, event)}
    />
    <Stack direction="row" spacing={1} alignItems="center" sx={{ mb: bundleUploadError ? 1 : 0, flexWrap: "wrap" }}>
      <Button size="small" variant="outlined" onClick={() => inputRef.current?.click()}>
        Upload Bundle
      </Button>
      {asArray(uploadedBundles).map((bundle) => (
        <Button
          key={bundle.id}
          size="small"
          variant="text"
          color="error"
          onClick={() => onRemoveBundle?.(state?.panel_id, bundle.id)}
        >
          Remove {bundle.label}
        </Button>
      ))}
    </Stack>
    {bundleUploadError ? <Alert severity="error">{bundleUploadError}</Alert> : null}
  </Box>
);

const TablePanel = ({
  title,
  state,
  onSelectRun = null,
  uploadedBundles = [],
  onUploadBundle = null,
  onRemoveBundle = null,
  bundleUploadError = null,
}) => {
  const uploadInputRef = useRef(null);
  const [sort, setSort] = useState(null);
  const [absolute, setAbsolute] = useState(false);
  const payload = state?.payload;
  const absoluteComponents = payload?.absolute_components;
  const activeData = absolute && absoluteComponents ? absoluteComponents : state;
  const columns = asArray(activeData?.columns);
  const rows = asArray(activeData?.rows);
  const displayRows = rows.map((row, rowIndex) => ({ row, rowIndex }));
  if (payload?.sortable && sort) {
    displayRows.sort((a, b) => compareTableValues(a.row[sort.column], b.row[sort.column], sort.direction));
  }
  const isHistogramBundle = payload?.histograms && typeof payload.histograms === "object" && !Array.isArray(payload.histograms);
  const actions = payload?.actions && typeof payload.actions === "object" ? payload.actions : {};
  const supportsBundleExport = actions.export === true || actions.export_json === true;
  const supportsBundleUpload = actions.upload_bundle === true;
  const omittedHistogramCount = asArray(payload?.omitted_incompatible_histograms).length;
  const rowAction = payload?.row_action && typeof payload.row_action === "object" ? payload.row_action : null;
  const rowTones = asArray(payload?.row_tones).map((tone) => (typeof tone === "string" ? tone : null));
  const rowToneLabels = payload?.row_tone_labels && typeof payload.row_tone_labels === "object" ? payload.row_tone_labels : {};
  const rowActionColumnIndex =
    rowAction?.kind === "select_run"
      ? columns.findIndex((column) => String(column || "").toLowerCase() === String(rowAction.column || "").toLowerCase())
      : -1;
  if (columns.length === 0 || rows.length === 0) {
    if (!isHistogramBundle) return null;
    return (
      <Card variant="outlined">
        <CardContent>
          <Typography variant="subtitle1" sx={{ mb: 1 }}>
            {title}
          </Typography>
          {supportsBundleUpload ? (
            <BundleUploadControls
              state={state}
              uploadedBundles={uploadedBundles}
              bundleUploadError={bundleUploadError}
              onUploadBundle={onUploadBundle}
              onRemoveBundle={onRemoveBundle}
              inputRef={uploadInputRef}
            />
          ) : null}
          <Alert severity="info">No observables available.</Alert>
        </CardContent>
      </Card>
    );
  }

  const columnKeys = columns.map((column) =>
    String(column || "")
      .trim()
      .toLowerCase(),
  );
  const visibleColumnIndices = (() => {
    const provided = asArray(state?.visible_column_indices)
      .map((value) => Number(value))
      .filter((index) => Number.isInteger(index) && index >= 0 && index < columns.length);
    if (provided.length === 0) return columns.map((_, index) => index);
    const deduplicated = provided.filter((index, position) => provided.indexOf(index) === position);
    return deduplicated.length > 0 ? deduplicated : columns.map((_, index) => index);
  })();
  const rowKeys = asArray(state?.row_keys).map((value) => String(value ?? ""));
  const centralValueIndex = columnKeys.findIndex((column) => column === "central value");
  const errorIndex = columnKeys.findIndex((column) => column === "dy" || column === "error");
  const selectableRows = rowKeys.length === rows.length;
  const rowsSelectRuns = typeof onSelectRun === "function" && rowActionColumnIndex >= 0;

  const handleDownload = async (format) => {
    if (!isHistogramBundle) return;
    try {
      const exported = await requestHistogramBundleExport(payload, format);
      const filename =
        typeof exported?.filename === "string" && exported.filename.trim().length > 0
          ? exported.filename
          : `${state?.panel_id ?? "histogram_bundle"}.${format === "hwu" ? "HwU" : "json"}`;
      const contents =
        typeof exported?.contents === "string"
          ? exported.contents
          : format === "json"
            ? `${JSON.stringify(payload, null, 2)}\n`
            : "";
      const mimeType =
        typeof exported?.mime_type === "string" && exported.mime_type.trim().length > 0
          ? exported.mime_type
          : format === "json"
            ? "application/json;charset=utf-8"
            : "text/plain;charset=utf-8";
      downloadTextFile(filename, contents, mimeType);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      alert(`Failed to export histogram bundle (${format.toUpperCase()}): ${message}`);
    }
  };

  const renderTableCell = (row, columnIndex) => {
    const value = row?.[columnIndex];
    if (value?.kind === "sample_point") return <SamplePointValue point={value} />;
    if (payload?.column_formats?.[columns[columnIndex]] === "scientific") {
      return <Box component="span" sx={{ whiteSpace: "nowrap" }}>{typeof value === "number" ? formatScientific(value) : renderStructuredValue(value)}</Box>;
    }
    if (columnIndex === centralValueIndex && errorIndex >= 0) {
      return formatCentralValueWithError(row?.[columnIndex], row?.[errorIndex], "n/a");
    }
    return renderStructuredValue(row?.[columnIndex]);
  };

  return (
    <Card variant="outlined">
      <CardContent>
        <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 2, mb: 2 }}>
          <Typography variant="subtitle1">{title}</Typography>
          {absoluteComponents ? (
            <Tooltip describeChild title="Means of |real| and |imag|, with their own errors. Variance contribution still refers to the signed campaign result.">
              <FormControlLabel
                sx={{ mr: 0 }}
                control={<Switch size="small" checked={absolute} onChange={(event) => setAbsolute(event.target.checked)} />}
                label="Absolute components"
              />
            </Tooltip>
          ) : null}
          {supportsBundleExport ? (
            <Stack direction="row" spacing={1} alignItems="center">
              {actions.export_json !== false ? (
              <Button size="small" variant="outlined" onClick={() => handleDownload("json")}>
                JSON
              </Button>
              ) : null}
              {actions.export_hwu !== false ? (
              <Button size="small" variant="outlined" onClick={() => handleDownload("hwu")}>
                HwU
              </Button>
              ) : null}
            </Stack>
          ) : null}
        </Box>
        {supportsBundleUpload ? (
          <BundleUploadControls
            state={state}
            uploadedBundles={uploadedBundles}
            bundleUploadError={bundleUploadError}
            onUploadBundle={onUploadBundle}
            onRemoveBundle={onRemoveBundle}
            inputRef={uploadInputRef}
          />
        ) : null}
        {omittedHistogramCount > 0 ? (
          <Alert severity="warning" sx={{ mb: 2 }}>
            {omittedHistogramCount} incompatible {omittedHistogramCount === 1 ? "observable was" : "observables were"}
            omitted from the combined result.
          </Alert>
        ) : null}
        <TableContainer sx={{ maxHeight: 440, overflowX: "auto" }}>
          <MuiTable size="small" stickyHeader sx={payload?.sortable ? { "& .MuiTableCell-root": { px: 1, whiteSpace: "nowrap" } } : undefined}>
            <TableHead>
              <TableRow>
                {payload?.row_numbers ? <TableCell sx={{ fontWeight: 600 }}>#</TableCell> : null}
                {visibleColumnIndices.map((columnIndex) => (
                  <TableCell
                    key={`${columns[columnIndex]}-${columnIndex}`}
                    sortDirection={payload?.sortable && sort?.column === columnIndex ? sort.direction : false}
                    sx={{ fontWeight: 600, whiteSpace: "nowrap" }}
                  >
                    {payload?.sortable ? (
                      <TableSortLabel
                        active={sort?.column === columnIndex}
                        direction={sort?.column === columnIndex ? sort.direction : "desc"}
                        onClick={() => setSort((previous) => ({
                          column: columnIndex,
                          direction: previous?.column === columnIndex && previous.direction === "desc" ? "asc" : "desc",
                        }))}
                      >
                        {columns[columnIndex]}
                      </TableSortLabel>
                    ) : columns[columnIndex]}
                  </TableCell>
                ))}
              </TableRow>
            </TableHead>
            <TableBody>
              {displayRows.map(({ row, rowIndex }, position) => {
                const rowTone = rowTones[rowIndex] || null;
                return (
                  <TableRow
                  key={`row-${rowIndex}`}
                  hover={selectableRows}
                  selected={
                    selectableRows &&
                    String(rowKeys[rowIndex] ?? "") ===
                      String(
                        isHistogramBundle
                          ? readHistogramBundleSelectedValue(state?.selected_value)
                          : state?.selected_value,
                      )
                  }
                  sx={{
                    cursor: selectableRows || rowsSelectRuns ? "pointer" : "default",
                    ...rowToneStyle(rowTone),
                  }}
                  onClick={
                    selectableRows
                      ? () =>
                          state?.onValueChange?.(
                            state?.panel_id,
                            isHistogramBundle
                              ? writeHistogramBundlePanelValue(state?.selected_value, {
                                  selectedHistogram: rowKeys[rowIndex],
                                })
                              : rowKeys[rowIndex],
                          )
                      : rowsSelectRuns
                        ? () => {
                            const runId = Number(row?.[rowActionColumnIndex]);
                            if (Number.isFinite(runId)) onSelectRun(runId);
                          }
                      : undefined
                  }
                  >
                  {payload?.row_numbers ? <TableCell>{position + 1}</TableCell> : null}
                  {visibleColumnIndices.map((columnIndex) => (
                    <TableCell
                      key={`${rowIndex}-${columnIndex}`}
                      sx={{
                        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, Liberation Mono, monospace",
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                        verticalAlign: "top",
                      }}
                    >
                      {columnIndex === visibleColumnIndices[0] && rowTone ? (
                        <Stack direction="row" spacing={1} alignItems="center" sx={{ flexWrap: "wrap" }}>
                          <Box component="span">{renderTableCell(row, columnIndex)}</Box>
                          <Chip
                            size="small"
                            color={rowToneChipColor(rowTone)}
                            label={rowToneLabels[rowTone] || rowTone}
                            sx={{ height: 20, fontSize: 11 }}
                          />
                        </Stack>
                      ) : (
                        renderTableCell(row, columnIndex)
                      )}
                    </TableCell>
                  ))}
                  </TableRow>
                );
              })}
            </TableBody>
          </MuiTable>
        </TableContainer>
      </CardContent>
    </Card>
  );
};

export default TablePanel;
