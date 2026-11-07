import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Checkbox,
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
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import LoadingStatus from "../common/LoadingStatus";
import { apiUrl } from "../../services/api";
import { asArray } from "../../utils/collections";
import { formatCentralValueWithError, formatScientific } from "../../utils/formatters";
import DisclosureField from "../common/DisclosureField";
import { fieldHelp } from "./fieldHelp";
import { formatF64Full } from "../../utils/formatters";
import { EstimateValueBlock, renderStructuredValue } from "./BasicPanels";
import SamplePointValue from "./SamplePointValue";
import StatusChip from "../common/StatusChip";
import { PanelHeader } from "../common/PanelChrome";
import PanelInfo from "../common/PanelInfo";
import FigureExportActions, { downloadBlob, downloadTextFile } from "./FigureExportActions";
import { readHistogramBundleSelectedValue, writeHistogramBundlePanelValue } from "./histogramUtils";

const requestHistogramBundleExport = async (payload, format, includeChildren) => {
  const source = payload?.export_source;
  const response = await fetch(apiUrl(source ? "/histogram-bundle/campaign-export" : "/histogram-bundle/export"), {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(source ? { ...source, format, include_children: includeChildren } : { payload, format }),
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

const BundleUploadControls = ({ state, uploadedBundles, bundleUploadError, bundleUploading, onUploadBundle, onRemoveBundle, inputRef, children }) => (
  <Box sx={{ mb: 1.5 }}>
    <input
      ref={inputRef}
      type="file"
      accept="application/json,.json,.hwu,.HwU,.HWU"
      aria-label="Upload histogram bundle"
      disabled={bundleUploading}
      style={{ display: "none" }}
      onChange={(event) => onUploadBundle?.(state?.panel_id, event)}
    />
    <Stack direction="row" spacing={1} useFlexGap alignItems="center" sx={{ mb: bundleUploadError ? 1 : 0, flexWrap: "wrap" }}>
      <Button size="small" variant="outlined" disabled={bundleUploading} onClick={() => inputRef.current?.click()}>
        {bundleUploading ? "Importing bundle…" : "Upload Bundle"}
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
      {children && <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap"
        sx={{ ml: "auto", maxWidth: "100%", justifyContent: "flex-end" }}>{children}</Stack>}
    </Stack>
    {bundleUploadError ? <Alert severity="error">{bundleUploadError}</Alert> : null}
    {asArray(uploadedBundles).flatMap((bundle) => asArray(bundle.warnings).map((warning, index) =>
      <Alert severity="info" key={`${bundle.id}-${index}`}>{bundle.label}: {warning}</Alert>))}
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
  bundleUploading = false,
}) => {
  const uploadInputRef = useRef(null);
  const [sort, setSort] = useState(null);
  const [absolute, setAbsolute] = useState(false);
  const [exporting, setExporting] = useState(null);
  const [exportError, setExportError] = useState(null);
  const [includeComparisons, setIncludeComparisons] = useState(false);
  const [includeChildren, setIncludeChildren] = useState(false);
  const [pdfProgress, setPdfProgress] = useState(null);
  const pdfRequest = useRef(null);
  useEffect(() => () => pdfRequest.current?.abort(), []);
  const payload = state?.payload;
  const maxWeightPoints = state?.panel_id === "max_weight_points";
  const weightBasis = maxWeightPoints && payload?.weight_basis?.startsWith("Pooled full-weight contribution") ? null : payload?.weight_basis;
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
  const hasRows = columns.length > 0 && rows.length > 0;
  if (!hasRows && !isHistogramBundle) return null;
  const bundleNotes = [weightBasis, payload?.estimator_note, payload?.description, payload?.help?.text,
    payload?.export_source && "Include child observables adds the original unscaled child observables to JSON/HwU downloads. Their data is fetched when you export."];

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
  })().filter((index) => !maxWeightPoints || !["top-level jacobian", "jacobian"].includes(columnKeys[index]));
  const rowKeys = asArray(state?.row_keys).map((value) => String(value ?? ""));
  const centralValueIndex = columnKeys.findIndex((column) => column === "central value");
  const errorIndex = columnKeys.findIndex((column) => column === "dy" || column === "error");
  const selectableRows = rowKeys.length === rows.length;
  const rowsSelectRuns = typeof onSelectRun === "function" && rowActionColumnIndex >= 0;

  const handleDownload = async (format) => {
    if (!isHistogramBundle || exporting) return;
    setExporting(format);
    setExportError(null);
    try {
      const exported = await requestHistogramBundleExport(payload, format, includeChildren);
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
      setExportError(`Failed to export histogram bundle (${format.toUpperCase()}): ${message}`);
    } finally {
      setExporting(null);
    }
  };

  const handleDownloadPdf = async () => {
    if (exporting) return;
    const request = new AbortController();
    pdfRequest.current = request;
    setExporting("pdf");
    setExportError(null);
    setPdfProgress(null);
    try {
      const { renderHistogramBundlePdf } = await import("./histogramPdf");
      if (request.signal.aborted) return;
      const blob = await renderHistogramBundlePdf({ payload, panelId: state.panel_id, value: state.selected_value,
        uploadedBundles, includeComparisons, signal: request.signal,
        onProgress: (page, total) => setPdfProgress(`Rendering observable ${page} of ${total}…`),
      });
      if (!request.signal.aborted) downloadBlob(`${state.panel_id || "observables"}.pdf`, blob);
    } catch (error) {
      if (!request.signal.aborted) setExportError(`Failed to export observable plots: ${error.message}`);
    } finally {
      if (!request.signal.aborted) { setExporting(null); setPdfProgress(null); }
      pdfRequest.current = null;
    }
  };

  const renderTableCell = (row, columnIndex) => {
    const value = row?.[columnIndex];
    if (value?.kind === "sample_point") return <SamplePointValue point={value} />;
    if (value?.kind === "estimate") return <EstimateValueBlock value={value} />;
    const help = payload?.column_help?.[columns[columnIndex]] || fieldHelp(columnKeys[columnIndex], columns[columnIndex], state?.panel_id);
    const precise = typeof value === "number" && !Number.isInteger(value);
    let displayed = renderStructuredValue(value);
    if (["state", "status"].includes(columnKeys[columnIndex]) && typeof value === "string") displayed = <StatusChip status={value} />;
    else if (payload?.column_formats?.[columns[columnIndex]] === "scientific" && typeof value === "number") displayed = formatScientific(value);
    else if (columnIndex === centralValueIndex && errorIndex >= 0) displayed = formatCentralValueWithError(value, row?.[errorIndex], "n/a");
    const numericStyle = typeof value === "number" ? { "& .field-value": { whiteSpace: "nowrap" } } : undefined;
    if (maxWeightPoints && columnKeys[columnIndex] === "max weighted value") {
      const point = row[columnKeys.indexOf("point")];
      const legacyJacobian = row[columnKeys.findIndex((key) => ["top-level jacobian", "jacobian"].includes(key))];
      const jacobian = point?.parameterization_jacobian ?? legacyJacobian;
      return <DisclosureField inline label="Max Weighted Value details" value={displayed} sx={numericStyle}>
        <Typography component="pre" variant="caption">{`Jacobian = ${formatF64Full(jacobian)}\nSampling weight = ${formatF64Full(point?.sampling_weight)}`}</Typography>
      </DisclosureField>;
    }
    return <DisclosureField inline label={`${columns[columnIndex]} details`} value={displayed} sx={numericStyle}>
      {help || precise ? <>
        {help && <Typography variant="body2" sx={{ mb: precise ? 1 : 0 }}>{help}</Typography>}
        {precise && <Typography component="pre" variant="caption">{formatF64Full(value)}</Typography>}
      </> : null}
    </DisclosureField>;
  };

  const bundleExports = supportsBundleExport && hasRows ? (
    <>
      {actions.export_json !== false && <Button size="small" variant="outlined" disabled={Boolean(exporting)} onClick={() => handleDownload("json")}>
        JSON
      </Button>}
      {actions.export_hwu !== false && <Button size="small" variant="outlined" disabled={Boolean(exporting)} onClick={() => handleDownload("hwu")}>
        HwU
      </Button>}
      <Button size="small" variant="outlined" disabled={Boolean(exporting)} onClick={handleDownloadPdf}>
        Download plots PDF
      </Button>
      {payload?.export_source && <FormControlLabel label="Include child observables" sx={{ m: 0 }}
        control={<Checkbox size="small" checked={includeChildren} disabled={Boolean(exporting)}
          onChange={(event) => setIncludeChildren(event.target.checked)} />} />}
      {uploadedBundles.length > 0 && <FormControlLabel label="Include comparisons in PDF" sx={{ m: 0 }}
        control={<Checkbox size="small" checked={includeComparisons} disabled={Boolean(exporting)}
          onChange={(event) => setIncludeComparisons(event.target.checked)} />} />}
    </>
  ) : null;

  return (
    <Card variant="outlined">
      <CardContent>
        {!isHistogramBundle && weightBasis && <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>{weightBasis}</Typography>}
        {!isHistogramBundle && payload?.estimator_note && <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 1 }}>{payload.estimator_note}</Typography>}
        <PanelHeader title={title}>
          {!isHistogramBundle && <FigureExportActions baseName={state?.panel_id || title} payload={{ kind: "table", state: { ...state, columns, rows } }} imageFormat={null} />}
          {absoluteComponents ? (
            <DisclosureField section label="Absolute components" ariaLabel="Absolute components explanation" actions={<Switch size="small" checked={absolute}
              slotProps={{ input: { role: "switch", "aria-label": "Absolute components" } }} onChange={(event) => setAbsolute(event.target.checked)} />}>
              <Typography variant="caption">Means of |real| and |imag|, with their own errors. Variance contribution still refers to the signed campaign result.</Typography>
            </DisclosureField>
          ) : null}
          {isHistogramBundle && <PanelInfo title={title} notes={bundleNotes} />}
        </PanelHeader>
        {supportsBundleUpload ? (
          <BundleUploadControls
            state={state}
            uploadedBundles={uploadedBundles}
            bundleUploadError={bundleUploadError}
            bundleUploading={bundleUploading}
            onUploadBundle={onUploadBundle}
            onRemoveBundle={onRemoveBundle}
            inputRef={uploadInputRef}
          >{bundleExports}</BundleUploadControls>
        ) : bundleExports && <Stack direction="row" spacing={1} alignItems="center" useFlexGap flexWrap="wrap" sx={{ mb: 1.5, justifyContent: "flex-end" }}>{bundleExports}</Stack>}
        {!isHistogramBundle && payload?.description && <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>{payload.description}</Typography>}
        {!isHistogramBundle && payload?.help && <DisclosureField section label={payload.help.title} sx={{ mb: 2 }}>
          <Typography variant="body2" color="text.secondary" sx={{ whiteSpace: "pre-line" }}>{payload.help.text}</Typography>
        </DisclosureField>}
        <LoadingStatus loading={Boolean(exporting)} label={pdfProgress || "Preparing export…"} />
        {exportError && <Alert severity="error">{exportError}</Alert>}
        {omittedHistogramCount > 0 ? (
          <Alert severity="warning" sx={{ mb: 2 }}>
            {omittedHistogramCount} incompatible {omittedHistogramCount === 1 ? "observable was" : "observables were"}
            omitted from the combined result.
          </Alert>
        ) : null}
        {!hasRows ? <Alert severity="info">No observables available.</Alert> : <TableContainer sx={{ maxHeight: 440, overflowX: "auto" }}>
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
        </TableContainer>}
      </CardContent>
    </Card>
  );
};

export default TablePanel;
