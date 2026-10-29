import { memo, useRef } from 'react';
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  MenuItem,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography
} from '@mui/material';
import LazyChart from './LazyChart';
import FigureExportActions from './FigureExportActions';
import HistogramPanel from './HistogramPanel';
import HeatmapColorLimitControl from './HeatmapColorLimitControl';
import { formatScientific } from '../../utils/formatters';
import { finite, prefixLabel, stratumColor, STRATUM_COLORS } from './strataUtils';
import { COVERAGE, diagnosticHistograms, pdfComparison, sharedColorLimits } from './pdfDiagnostics';
import { lineComparisonOption, planeComparisonOption } from './pdfPlotOptions';
import {
  ScopeSelector,
  STRATA_PAGE_SIZE,
  navigationEvents,
  scopeIndices
} from './DiagnosticControls';
import { FULL_ZOOM } from './panelView';
const numeric = (v) => (finite(v) ? formatScientific(v, 5) : 'Unavailable');
const percent = (v) => (finite(v) ? `${(v * 100).toFixed(3)}%` : 'Unavailable');
const Figure = ({ name, option, payload, onEvents }) => {
  const chart = useRef(null),
    element = useRef(null);
  return (
    <Box
      ref={element}
      sx={{
        minWidth: 0
      }}
    >
      <Stack
        direction="row"
        justifyContent="space-between"
        alignItems="center"
        flexWrap="wrap"
        gap={1}
      >
        <Typography variant="subtitle2">{name}</Typography>
        <FigureExportActions
          baseName={name}
          payload={payload}
          echartsRef={chart}
          elementRef={element}
        />
      </Stack>
      <LazyChart
        ref={chart}
        option={option}
        style={{
          height: 360
        }}
        onEvents={onEvents}
        notMerge
      />
    </Box>
  );
};
const PdfComparisonPanel = memo(function PdfComparisonPanel({ title, state, value = {}, onValueChange }) {
  const prefixes = state.data.strata?.prefixes || [];
  const scope = prefixes.length ? (value?.scope ?? 'overall') : 'overall';
  const indices = scopeIndices(scope, prefixes.length, value?.page);
  const choices = scope === 'overall' ? ['overall'] : indices.map(String);
  const line = state.geometry.kind === 'line';
  const view = [
    'density',
    'mismatch',
    'histogram',
    'coverage',
    ...(prefixes.length ? ['membership'] : [])
  ].includes(value?.view)
    ? value.view
    : 'density';
  const hasReference =
    finite(state.data.global_abs_integrand_norm) && state.data.global_abs_integrand_norm > 0;
  const requested = ['shape', 'conditional', 'joint', 'reference'].includes(value?.normalization)
    ? value.normalization
    : 'shape';
  const mode =
    (!hasReference && requested === 'reference') ||
    (['mismatch', 'histogram'].includes(view) && ['conditional', 'joint'].includes(requested))
      ? 'shape'
      : requested;
  const metric = value?.metric === 'relative' ? 'relative' : 'log_ratio';
  const log = view === 'density' && Boolean(value?.log);
  const stratumColours = !line && view === 'density' && prefixes.length > 0 && value?.colourEncoding === 'strata';
  const update = (patch) =>
    onValueChange?.(
      state.panel_id,
      {
        ...value,
        ...patch
      },
      false
    );
  const compared = choices.map((k) => pdfComparison(state, k, mode));
  const projection = state.data.target_label || 'Absolute source training projection';
  const events = navigationEvents(value, update);
  const changeView = (next) =>
    update({
      view: next,
      normalization:
        ['mismatch', 'histogram'].includes(next) && ['conditional', 'joint'].includes(mode)
          ? 'shape'
          : mode
    });
  const rawDensity = view === 'density' && ['conditional', 'joint'].includes(mode);
  const colorContext = `${view}:${mode}:${log}:${metric}`;
  const limitsFor = (role) =>
    sharedColorLimits(
      compared.flatMap((c) =>
        view === 'mismatch'
          ? [metric === 'log_ratio' ? c.logRatio : c.relative]
          : rawDensity
            ? [role === 'target' ? c.target : c.pdf]
            : [c.target, c.pdf]
      ),
      {
        log,
        symmetric: view === 'mismatch',
        manual: value?.colorLimits?.context === colorContext ? value.colorLimits?.[role] : null
      }
    );
  const limits = limitsFor('pdf'),
    targetLimits = rawDensity ? limitsFor('target') : limits;
  const details =
    scope === 'overall' && prefixes.length
      ? prefixes.slice(0, STRATA_PAGE_SIZE).map((_, k) => pdfComparison(state, String(k), mode))
      : compared;
  const summaries = compared.map((c) => ({
    scope: c.scope,
    prefix: c.k == null ? null : prefixes[c.k],
    probability: c.probability,
    target_integral: c.targetIntegral,
    pdf_integral: c.pdfIntegral,
    grid_pdf_integral: c.gridPdfIntegral,
    pdf_integral_method: c.pdfIntegralMethod,
    pdf_divisor: c.pdfDivisor,
    target_divisor: c.targetDivisor,
    target_share: c.targetShare,
    actual_cell_mass: c.actualMass,
    leakage: c.leakage,
    counts: c.counts
  }));
  const rawExport = {
    ...state,
    view: {
      ...value,
      scope,
      view,
      normalization: mode
    },
    diagnostics: summaries
  };
  const exportFor = (c) => ({
    normalization: mode,
    colour_encoding: stratumColours ? 'stratum_hue_density' : 'density',
    density_scale: log ? 'logarithmic' : 'linear',
    scope: c.scope,
    prefix: c.k == null ? null : prefixes[c.k],
    geometry: state.geometry,
    pdf_steps: c.stepPoints,
    points: c.axes.x.flatMap((x, ix) =>
      (c.axes.y || [null]).map((y, iy) => {
        const i = iy * c.axes.x.length + ix;
        return {
          x,
          y,
          target: c.target[i],
          pdf: c.pdf[i],
          relative_mismatch: c.relative[i],
          log_ratio: c.logRatio[i],
          coverage: c.status[i],
          actual_stratum: c.actual[i] ?? null
        };
      })
    )
  });
  return (
    <Card>
      <CardContent>
        <Stack spacing={2}>
          <Stack
            direction="row"
            justifyContent="space-between"
            alignItems="center"
            flexWrap="wrap"
            gap={1}
          >
            <Typography variant="h6">{title}</Typography>
            <FigureExportActions baseName="pdf-comparison" payload={rawExport} imageFormat={null} />
          </Stack>
          <Stack direction="row" gap={2} flexWrap="wrap" alignItems="center">
            <ScopeSelector prefixes={prefixes} scope={scope} page={value?.page} onChange={update} />
            <TextField
              select
              label="View"
              size="small"
              sx={{
                minWidth: 180
              }}
              value={view}
              onChange={(e) => changeView(e.target.value)}
            >
              <MenuItem value="density">Density comparison</MenuItem>
              <MenuItem value="mismatch">Sampling mismatch</MenuItem>
              <MenuItem value="histogram">Distributions</MenuItem>
              <MenuItem value="coverage">Coverage</MenuItem>
              {prefixes.length > 0 && <MenuItem value="membership">Stratum membership</MenuItem>}
            </TextField>
            {!['coverage', 'membership'].includes(view) && (
              <TextField
                select
                label="Normalization"
                size="small"
                sx={{
                  minWidth: 225
                }}
                value={mode}
                onChange={(e) =>
                  update({
                    normalization: e.target.value
                  })
                }
              >
                <MenuItem value="shape">Shape on this slice</MenuItem>
                <MenuItem value="reference" disabled={!hasReference}>
                  Source integral reference
                </MenuItem>
                {view === 'density' && <MenuItem value="conditional">Conditional density</MenuItem>}
                {view === 'density' && <MenuItem value="joint">Joint density</MenuItem>}
              </TextField>
            )}
            {view === 'density' && (
              <TextField
                select
                label="Density scale"
                size="small"
                sx={{
                  minWidth: 160
                }}
                value={log ? 'log' : 'linear'}
                onChange={(e) =>
                  update({
                    log: e.target.value === 'log'
                  })
                }
              >
                <MenuItem value="linear">Linear</MenuItem>
                <MenuItem value="log">Logarithmic</MenuItem>
              </TextField>
            )}
            {view === 'mismatch' && (
              <TextField
                select
                label="Metric"
                size="small"
                value={metric}
                onChange={(e) =>
                  update({
                    metric: e.target.value
                  })
                }
              >
                <MenuItem value="relative">PDF / target − 1</MenuItem>
                <MenuItem value="log_ratio">log₁₀(PDF / target)</MenuItem>
              </TextField>
            )}
            {view === 'histogram' && (
              <TextField
                select
                label="Quantity"
                size="small"
                value={value?.histogramQuantity || 'density'}
                onChange={(e) =>
                  update({
                    histogramQuantity: e.target.value,
                    histogram: {}
                  })
                }
              >
                <MenuItem value="density">Target and PDF values</MenuItem>
                <MenuItem value="relative">Relative mismatch</MenuItem>
                <MenuItem value="log_ratio">Log ratio</MenuItem>
              </TextField>
            )}
            {!line && view === 'density' && prefixes.length > 0 && (
              <TextField
                select
                label="Colour encoding"
                size="small"
                sx={{ minWidth: 225 }}
                value={stratumColours ? 'strata' : 'density'}
                onChange={(e) => update({ colourEncoding: e.target.value })}
              >
                <MenuItem value="density">Density only</MenuItem>
                <MenuItem value="strata">Stratum hue + density</MenuItem>
              </TextField>
            )}
            <Button
              onClick={() =>
                update({
                  zoom: FULL_ZOOM,
                  yZoom: FULL_ZOOM,
                  histogram: {}
                })
              }
            >
              Reset view
            </Button>
          </Stack>
          {stratumColours && (
            <Stack spacing={1}>
              <Typography variant="caption">
                Hue shows the actual integrand stratum in both plots. Saturation increases with
                density on the selected scale; white is the low end. Sampler density in another
                stratum's colour shows leakage. Unknown membership is grey.
              </Typography>
              <Stack direction="row" gap={2} flexWrap="wrap">
                {prefixes.slice(0, STRATUM_COLORS.length).map((prefix, k) => (
                  <Stack key={prefix.join(',')} direction="row" gap={0.5} alignItems="center">
                    <Box sx={{ width: 12, height: 12, bgcolor: stratumColor(k) }} />
                    <Typography variant="caption">{prefixLabel(prefix)}</Typography>
                  </Stack>
                ))}
              </Stack>
              {prefixes.length > STRATUM_COLORS.length && (
                <Typography variant="caption">
                  Colours repeat after {STRATUM_COLORS.length} strata. Hover for the exact prefix or use Scope.
                </Typography>
              )}
            </Stack>
          )}
          <Typography variant="body2">
            Target: {projection}.{' '}
            {['coverage', 'membership'].includes(view)
              ? 'Coverage and membership use the raw evaluated values.'
              : mode === 'shape'
                ? 'Each density is normalized over the displayed parameter slice.'
                : mode === 'reference'
                  ? 'The target is divided by the source estimate of its full-domain absolute integral; the sampler remains a joint density.'
                  : 'The PDF is raw; the unnormalized importance target uses its own scale.'}{' '}
            These are slices, not marginals. Zooming does not change normalization.
          </Typography>
          {view === 'density' && (
            <Typography variant="caption" color="text.secondary">
              {state.data.pdf_steps
                ? 'Native Havana densities use exact bin edges and bin integrals.'
                : line
                  ? 'PDF curves interpolate sampled grid values.'
                  : 'Heatmaps show queried grid values.'}{' '}
              {log
                ? 'Zeros are omitted on logarithmic scales; Coverage distinguishes zero and unavailable values.'
                : ''}
            </Typography>
          )}
          {view === 'mismatch' && metric === 'relative' && (
            <Typography variant="caption" color="text.secondary">
              Near-zero targets can produce very large relative mismatches. Use log ratio or the
              vertical zoom to inspect smaller differences.
            </Typography>
          )}
          {!line && ['density', 'mismatch'].includes(view) && (
            <Accordion disableGutters>
              <AccordionSummary>Colour limits · linked across comparable plots</AccordionSummary>
              <AccordionDetails>
                <Stack spacing={2}>
                  {(rawDensity ? ['pdf', 'target'] : ['pdf']).map((role) => {
                    const l = role === 'target' ? targetLimits : limits;
                    const set = (side, v) =>
                      update({
                        colorLimits: {
                          context: colorContext,
                          ...(value?.colorLimits?.context === colorContext
                            ? value.colorLimits
                            : {}),
                          [role]: {
                            min: l.min,
                            max: l.max,
                            [side]: v
                          }
                        }
                      });
                    return (
                      <Box key={role}>
                        <Typography variant="body2">
                          {rawDensity
                            ? role === 'target'
                              ? 'Importance target'
                              : 'Sampler density'
                            : 'Shared scale'}
                          {log ? ' (log₁₀ limits)' : ''}
                        </Typography>
                        <Stack direction="row" gap={2} flexWrap="wrap">
                          <HeatmapColorLimitControl
                            side="min"
                            limit={l.min}
                            otherLimit={l.max}
                            defaultScale={l.auto}
                            zeroOrigin={false}
                            onChange={(v) => set('min', v)}
                          />
                          <HeatmapColorLimitControl
                            side="max"
                            limit={l.max}
                            otherLimit={l.min}
                            defaultScale={l.auto}
                            zeroOrigin={false}
                            onChange={(v) => set('max', v)}
                          />
                        </Stack>
                      </Box>
                    );
                  })}
                  <Button
                    onClick={() =>
                      update({
                        colorLimits: null
                      })
                    }
                  >
                    Auto limits
                  </Button>
                </Stack>
              </AccordionDetails>
            </Accordion>
          )}
          {view === 'membership' && prefixes.length > 8 && scope === 'overall' && (
            <Typography variant="caption">
              Hover for full prefix labels, or select a stratum to highlight it. Colours repeat for
              large partitions.
            </Typography>
          )}
          <Stack spacing={3}>
            {compared.map((c) => {
              const name = c.k == null ? 'Overall' : `Stratum ${prefixLabel(prefixes[c.k])}`;
              const states = COVERAGE.filter((s) => s.id !== 'covered' && c.counts[s.id] > 0)
                .map((s) => `${s.label}: ${c.counts[s.id]}`)
                .join(' · ');
              const numericalMissing = c.status.filter(
                (status, i) =>
                  status === 'covered' &&
                  !finite((metric === 'relative' ? c.relative : c.logRatio)[i])
              ).length;
              const payload = exportFor(c);
              const categories =
                view === 'membership'
                  ? c.k == null
                    ? prefixes.map((p, k) => ({
                        label: prefixLabel(p),
                        color: stratumColor(k)
                      }))
                    : [
                        {
                          label: 'Other strata',
                          color: '#d8dde3'
                        },
                        {
                          label: prefixLabel(prefixes[c.k]),
                          color: stratumColor(c.k)
                        }
                      ]
                  : COVERAGE;
              const categorical = ['coverage', 'membership'].includes(view);
              const categoryValues =
                view === 'membership'
                  ? c.actual.map((k) => (k == null ? null : c.k == null ? k : Number(k === c.k)))
                  : c.status.map((s) => COVERAGE.findIndex((v) => v.id === s));
              const histogramQuantity = value?.histogramQuantity || 'density';
              const histValues =
                histogramQuantity === 'density'
                  ? [c.target, c.pdf]
                  : [histogramQuantity === 'relative' ? c.relative : c.logRatio];
              const hist = view === 'histogram' ? diagnosticHistograms(histValues) : null;
              return (
                <Box
                  key={c.scope}
                  sx={{
                    minWidth: 0
                  }}
                >
                  <Typography variant="subtitle1">
                    {name}
                    {c.k != null ? ` · p = ${numeric(c.probability)}` : ''}
                  </Typography>
                  {view === 'mismatch' && numericalMissing > 0 && (
                    <Typography variant="caption" color="text.secondary">
                      Numerically unavailable ratios: {numericalMissing}. Check normalization or use
                      log ratio to avoid division overflow.
                    </Typography>
                  )}
                  {states && (
                    <Typography variant="body2" color="text.secondary">
                      Grid points — {states}.{' '}
                      {view === 'mismatch' || view === 'histogram'
                        ? 'Undefined/infinite ratios are excluded from numeric curves and remain visible in Coverage.'
                        : ''}
                    </Typography>
                  )}
                  {['density', 'mismatch'].includes(view) &&
                    (!c.pdf.some(finite) || !c.target.some(finite)) && (
                      <Alert severity="info">
                        Normalization or density unavailable: check completion, positive slice
                        integrals and exact prefix masses in the details below.
                      </Alert>
                    )}
                  {view === 'histogram' ? (
                    <>
                      <Typography variant="caption">
                        Distribution across deterministic grid points. No Monte Carlo error bars.{' '}
                        {histogramQuantity === 'density'
                          ? ''
                          : 'Nonfinite ratios are counted above.'}
                      </Typography>
                      <HistogramPanel
                        title={`${name} — ${histogramQuantity === 'density' ? 'Target / PDF distributions' : 'Mismatch distribution'}`}
                        state={{
                          kind: 'histogram',
                          panel_id: `${state.panel_id}-histogram`,
                          bins: hist[0],
                          controls: {
                            scale: true,
                            x_scale: true,
                            pdf_cdf: true,
                            ratio: true,
                            relative_error: false,
                            export: true,
                            reset_view: true,
                            sort: false
                          },
                          overlay_alignment: 'shared_edges',
                          overlay_histograms:
                            hist.length > 1
                              ? [
                                  {
                                    name: 'Sampler PDF',
                                    bins: hist[1],
                                    suppress_error_bars: true
                                  }
                                ]
                              : []
                        }}
                        value={value?.histogram || {}}
                        onValueChange={(_id, next) =>
                          update({
                            histogram: next
                          })
                        }
                      />
                    </>
                  ) : line ? (
                    <Figure
                      name={name}
                      option={lineComparisonOption(c, {
                        view,
                        metric,
                        mode,
                        log,
                        prefixes,
                        value
                      })}
                      payload={payload}
                      onEvents={events}
                    />
                  ) : categorical ? (
                    <Figure
                      name={`${name} — ${view}`}
                      option={planeComparisonOption(c, categoryValues, {
                        label: view,
                        categories,
                        value,
                        prefixes
                      })}
                      payload={payload}
                      onEvents={events}
                    />
                  ) : view === 'mismatch' ? (
                    <Figure
                      name={`${name} — mismatch`}
                      option={planeComparisonOption(
                        c,
                        metric === 'relative' ? c.relative : c.logRatio,
                        {
                          label: metric === 'relative' ? 'PDF / target − 1' : 'log₁₀(PDF / target)',
                          limits,
                          value,
                          mismatch: true,
                          prefixes
                        }
                      )}
                      payload={payload}
                      onEvents={events}
                    />
                  ) : (
                    <Box
                      sx={{
                        display: 'grid',
                        gridTemplateColumns: {
                          xs: '1fr',
                          md: '1fr 1fr'
                        },
                        gap: 2
                      }}
                    >
                      <Figure
                        name={`${name} — target`}
                        option={planeComparisonOption(c, c.target, {
                          label: 'Target',
                          limits: targetLimits,
                          log,
                          value,
                          prefixes,
                          stratumColours
                        })}
                        payload={payload}
                        onEvents={events}
                      />
                      <Figure
                        name={`${name} — sampler`}
                        option={planeComparisonOption(c, c.pdf, {
                          label: 'Sampler density',
                          limits,
                          log,
                          value,
                          prefixes,
                          stratumColours
                        })}
                        payload={payload}
                        onEvents={events}
                      />
                    </Box>
                  )}
                </Box>
              );
            })}
          </Stack>
          <Accordion disableGutters>
            <AccordionSummary>Normalization and leakage details</AccordionSummary>
            <AccordionDetails>
              <Stack spacing={2}>
                <Typography variant="body2">
                  Source estimate of ∫ target: {numeric(state.data.global_abs_integrand_norm)}. This
                  is an absolute-projection integral, not the magnitude of a signed integral.
                </Typography>
                <Box
                  sx={{
                    overflowX: 'auto'
                  }}
                >
                  <Table size="small">
                    <TableHead>
                      <TableRow>
                        {[
                          'Scope',
                          'Raw target slice integral',
                          'PDF slice integral',
                          'Grid PDF integral',
                          'Target divisor',
                          'PDF divisor'
                        ].map((s) => (
                          <TableCell key={s}>{s}</TableCell>
                        ))}
                      </TableRow>
                    </TableHead>
                    <TableBody>
                      {compared.map((c) => (
                        <TableRow key={c.scope}>
                          <TableCell>
                            {c.k == null ? 'Overall' : prefixLabel(prefixes[c.k])}
                          </TableCell>
                          <TableCell>{numeric(c.targetIntegral)}</TableCell>
                          <TableCell>
                            {numeric(c.pdfIntegral)} · {c.pdfIntegralMethod}
                          </TableCell>
                          <TableCell>{numeric(c.gridPdfIntegral)}</TableCell>
                          <TableCell>{numeric(c.targetDivisor)}</TableCell>
                          <TableCell>{numeric(c.pdfDivisor)}</TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </Box>
                {prefixes.length > 0 && (
                  <Box
                    sx={{
                      overflowX: 'auto'
                    }}
                  >
                    <Table size="small">
                      <TableHead>
                        <TableRow>
                          {[
                            'Stratum',
                            'Exact requested p',
                            'Target share on slice',
                            'Mixture mass in actual cell',
                            'Proposal outside intended cell'
                          ].map((s) => (
                            <TableCell key={s}>{s}</TableCell>
                          ))}
                        </TableRow>
                      </TableHead>
                      <TableBody>
                        {details.map((c) => (
                          <TableRow key={c.scope}>
                            <TableCell>
                              {c.k == null ? 'Overall' : prefixLabel(prefixes[c.k])}
                            </TableCell>
                            <TableCell>{percent(c.probability)}</TableCell>
                            <TableCell>{percent(c.targetShare)}</TableCell>
                            <TableCell>{percent(c.actualMass)}</TableCell>
                            <TableCell>{percent(c.leakage)}</TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                    <Typography variant="caption">
                      Cell masses and leakage are grid estimates on this slice, not full-domain
                      probabilities.{' '}
                      {scope === 'overall' && prefixes.length > STRATA_PAGE_SIZE
                        ? `Showing the first ${STRATA_PAGE_SIZE} strata; use Scope to inspect the others.`
                        : ''}
                    </Typography>
                  </Box>
                )}
                <Typography variant="caption">
                  Boundary positions are estimated from adjacent classified grid points; narrow or
                  unsampled regions can be missed. Sampler densities remain unmasked outside their
                  intended strata.
                </Typography>
              </Stack>
            </AccordionDetails>
          </Accordion>
        </Stack>
      </CardContent>
    </Card>
  );
});

export default PdfComparisonPanel;
