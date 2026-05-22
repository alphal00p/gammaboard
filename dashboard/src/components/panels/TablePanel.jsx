import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Stack,
  Table as MuiTable,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { useRef } from "react";
import { asArray } from "../../utils/collections";
import { formatCentralValueWithError } from "../../utils/formatters";
import { renderStructuredValue } from "./BasicPanels";
import { downloadTextFile } from "./FigureExportActions";
import { readHistogramBundleSelectedValue, writeHistogramBundlePanelValue } from "./histogramUtils";

const requestHistogramBundleExport = async (payload, format) => {
  const response = await fetch("/api/histogram-bundle/export", {
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
  const columns = asArray(state?.columns);
  const rows = asArray(state?.rows);
  const payload = state?.payload;
  const isHistogramBundle = payload?.histograms && typeof payload.histograms === "object" && !Array.isArray(payload.histograms);
  const actions = payload?.actions && typeof payload.actions === "object" ? payload.actions : {};
  const supportsBundleExport = actions.export === true || actions.export_json === true;
  const supportsBundleUpload = actions.upload_bundle === true;
  const rowAction = payload?.row_action && typeof payload.row_action === "object" ? payload.row_action : null;
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
          <Alert severity="info">Bundle is empty.</Alert>
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
        <TableContainer sx={{ maxHeight: 440, overflowX: "auto" }}>
          <MuiTable size="small" stickyHeader>
            <TableHead>
              <TableRow>
                {visibleColumnIndices.map((columnIndex) => (
                  <TableCell key={`${columns[columnIndex]}-${columnIndex}`} sx={{ fontWeight: 600, whiteSpace: "nowrap" }}>
                    {columns[columnIndex]}
                  </TableCell>
                ))}
              </TableRow>
            </TableHead>
            <TableBody>
              {rows.map((row, rowIndex) => (
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
                  sx={{ cursor: selectableRows || rowsSelectRuns ? "pointer" : "default" }}
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
                      {renderTableCell(row, columnIndex)}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </MuiTable>
        </TableContainer>
      </CardContent>
    </Card>
  );
};

export default TablePanel;
