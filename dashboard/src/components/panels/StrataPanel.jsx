import { PanelHeader, ControlBar } from "../common/PanelChrome";
import { useEffect, useRef } from 'react';
import {
  Box,
  Card,
  CardContent,
  Checkbox,
  FormControlLabel,
  MenuItem,
  Stack,
  TextField,
  Typography
} from '@mui/material';
import DisclosureField from "../common/DisclosureField";
import { fieldHelp } from "./fieldHelp";
import LazyChart from './LazyChart';
import FigureExportActions, { escapeXml } from './FigureExportActions';
import {
  DiagnosticSelect,
  ScopeSelector,
  scopeIndices,
  navigationZoom,
  navigationEvents
} from './DiagnosticControls';
import { FULL_ZOOM } from './panelView';
import { buildErrorBarSeries, formatAxisValue } from './chartPrimitives';
import { EstimateValueBlock } from "./BasicPanels";
import { formatScientific } from '../../utils/formatters';
import {
  finite,
  fraction,
  prefixLabel,
  stratumColor,
  stratumHistory,
  varianceShares
} from './strataUtils';
export const uncertaintyBand = (name, points, color) => ({
  name,
  type: 'custom',
  silent: true,
  clip: true,
  tooltip: {
    show: false
  },
  data: points
    .slice(1)
    .map((p, i) => [points[i].x, points[i].y, points[i].error, p.x, p.y, p.error])
    .filter((d) => d.every(finite)),
  renderItem: (_params, api) => ({
    type: 'polygon',
    shape: {
      points: [
        api.coord([api.value(0), api.value(1) - api.value(2)]),
        api.coord([api.value(0), api.value(1) + api.value(2)]),
        api.coord([api.value(3), api.value(4) + api.value(5)]),
        api.coord([api.value(3), api.value(4) - api.value(5)])
      ]
    },
    style: {
      fill: color,
      opacity: 0.14
    }
  })
});
const PercentRow = ({ label, values, rows }) => (
  <Box>
    <DisclosureField section label={label}>
      <Typography variant="caption">{fieldHelp("", label, "strata")}</Typography>
    </DisclosureField>
    <Box
      sx={{
        display: 'flex',
        gap: 1,
        flexWrap: 'wrap'
      }}
    >
      {rows.map((row, k) => (
          <Box key={prefixLabel(row.prefix)} sx={{ flex: '1 1 180px', minWidth: 160 }}>
            <DisclosureField label={prefixLabel(row.prefix)} value={finite(values[k]) ? `${(100 * values[k]).toFixed(2)}%` : 'unavailable'}
              sx={{ "--field-label-width": "minmax(0, 1fr)" }}>
              <Typography variant="caption">{row.attempts} attempts, {row.matches} matches. Full fraction: {finite(values[k]) ? values[k].toExponential(16) : 'unavailable'}.</Typography>
            </DisclosureField>
            <Box
              sx={{
                height: 7,
                bgcolor: 'action.hover',
                mt: 0.3
              }}
            >
              <Box
                sx={{
                  height: '100%',
                  width: `${finite(values[k]) ? 100 * values[k] : 0}%`,
                  bgcolor: stratumColor(row.colorIndex ?? k)
                }}
              />
            </Box>
          </Box>
      ))}
    </Box>
  </Box>
);
export default function StrataPanel({ title, state, value = {}, onValueChange }) {
  const ref = useRef(null),
    panel = useRef(null),
    previousExtent = useRef(null);
  const selectedAxis = value?.axis || 'samples';
  const timeField = selectedAxis === 'wall' ? 'wall_time_ms' : 'sampler_uptime_ms';
  const timed = state.frames?.filter((f) => finite(f[timeField])) || [];
  const extent =
    selectedAxis === 'samples'
      ? state.frames?.at(-1)?.samples || 0
      : timed.length
        ? (timed.at(-1)[timeField] - timed[0][timeField]) / 1000
        : 0;
  useEffect(() => {
    const previous = previousExtent.current;
    previousExtent.current = {
      axis: selectedAxis,
      max: extent
    };
    if (
      previous?.axis === selectedAxis &&
      previous.max > 0 &&
      extent > previous.max &&
      value?.follow !== false &&
      value?.zoom &&
      value.zoom.start > 0
    ) {
      const span = ((value.zoom.end - value.zoom.start) * previous.max) / extent;
      onValueChange?.(
        state.panel_id,
        {
          ...value,
          zoom: {
            start: Math.max(0, 100 - span),
            end: 100
          }
        },
        false
      );
    }
  }, [extent, selectedAxis, onValueChange, state.panel_id, value]);
  const latest = state.frames?.at(-1);
  if (!latest) return null;
  const component = Math.min(Number(value?.component) || 0, state.components.length - 1);
  const conditional = state.estimator === 'stratified';
  const metric = value?.metric || (conditional ? 'contribution' : 'allocation');
  const activeMetric = !conditional && metric === 'contribution' ? 'allocation' : metric;
  const interval = Boolean(value?.interval);
  const update = (patch) =>
    onValueChange?.(
      state.panel_id,
      {
        ...value,
        ...patch
      },
      false
    );
  const scope = value?.scope || 'compare';
  const indices = scopeIndices(scope, latest.rows.length, value?.page);
  const rows = indices.map((i) => ({
    ...latest.rows[i],
    colorIndex: i
  }));
  const axis = value?.axis || 'samples';
  const timeKey = axis === 'wall' ? 'wall_time_ms' : 'sampler_uptime_ms';
  const origin = state.frames.find((f) => finite(f[timeKey]))?.[timeKey];
  const remap = (p) => ({
    ...p,
    x: axis === 'samples' ? p.x : finite(p[timeKey]) ? (p[timeKey] - origin) / 1000 : null
  });
  const series = stratumHistory(state, component, activeMetric, interval)
    .filter((_, i) => indices.includes(i))
    .map((s) => ({
      ...s,
      points: s.points.map(remap).filter((p) => p.x != null)
    }));
  const total = latest.total?.[component];
  const totalPoints = state.frames
    .map((f) =>
      remap({
        x: f.samples,
        wall_time_ms: f.wall_time_ms,
        sampler_uptime_ms: f.sampler_uptime_ms,
        y: f.total?.[component]?.mean ?? null,
        error: f.total?.[component]?.error ?? null
      })
    )
    .filter((p) => p.x != null);
  const drawTotal = conditional && activeMetric === 'contribution' && value?.total !== false;
  const visible = drawTotal
    ? [
        ...series,
        {
          name: 'Total (all strata)',
          color: '#818b98',
          points: totalPoints
        }
      ]
    : series;
  const option = {
    animation: false,
    grid: {
      left: 76,
      right: 24,
      top: 52,
      bottom: 62
    },
    legend: {
      top: 0,
      bottom: 'auto',
      type: 'scroll',
      data: visible.map((s) => s.name)
    },
    tooltip: {
      trigger: 'axis',
      formatter: (items) =>
        items
          .filter((p) => p.seriesType === 'line')
          .map(
            (p) =>
              `${escapeXml(p.seriesName)}: ${finite(p.value[1]) ? formatScientific(p.value[1], 8) : 'unavailable'}${activeMetric === 'contribution' ? ` ± ${finite(p.value[2]) ? formatScientific(p.value[2], 8) : 'unavailable'}` : ''}`
          )
          .join('<br/>')
    },
    xAxis: {
      type: 'value',
      splitNumber: 3,
      name:
        axis === 'samples'
          ? 'Completed samples'
          : axis === 'wall'
            ? 'Elapsed seconds since first snapshot'
            : 'Sampler seconds since first report',
      nameLocation: 'middle',
      nameGap: 30,
      axisLabel: {
        hideOverlap: true,
        formatter: formatAxisValue
      },
    },
    yAxis: {
      type: 'value',
      name:
        activeMetric === 'contribution'
          ? `Integral contribution (${state.components[component]})`
          : activeMetric === 'allocation'
            ? 'Allocation'
            : 'Hit rate',
      min: activeMetric === 'contribution' ? null : 0,
      max: activeMetric === 'contribution' ? null : 1,
      axisLabel: {
        formatter:
          activeMetric === 'contribution' ? formatAxisValue : (v) => `${(v * 100).toFixed(0)}%`
      }
    },
    dataZoom: navigationZoom(value),
    series: visible.flatMap((s) => [
      {
        type: 'line',
        name: s.name,
        data: s.points.map((p) => [p.x, p.y, p.error]),
        showSymbol: s.points.length === 1,
        connectNulls: false,
        itemStyle: {
          color: s.color
        },
        lineStyle: {
          width: s.name.startsWith('Total') ? 3 : 1.5
        }
      },
      uncertaintyBand(s.name, s.points, s.color),
      ...(s.points.length === 1
        ? [
            buildErrorBarSeries({
              name: s.name,
              color: s.color,
              data: s.points
                .filter((p) => finite(p.y) && finite(p.error))
                .map((p) => [p.x, p.y - p.error, p.y + p.error])
            })
          ]
        : [])
    ])
  };
  return (
    <Card ref={panel}>
      <CardContent>
        <Stack spacing={2}>
          <PanelHeader title={title} sx={{ mb: 0 }}>
            <FigureExportActions
              baseName="strata"
              payload={{
                ...state,
                view: value
              }}
              elementRef={panel}
              echartsRef={ref}
              onResetView={() => update({ zoom: FULL_ZOOM, yZoom: FULL_ZOOM, follow: true })}
            />
          </PanelHeader>
          <ControlBar>
            <ScopeSelector
              prefixes={latest.rows.map((r) => r.prefix)}
              scope={scope}
              page={value?.page}
              onChange={update}
              overall={false}
            />
            {conditional && state.components.length > 1 && (
              <DiagnosticSelect
                label="Component"
                value={String(component)}
                onChange={(next) =>
                  update({
                    component: Number(next)
                  })
                }
                options={state.components.map((c, i) => ({
                  value: String(i),
                  label: c
                }))}
                minWidth={130}
              />
            )}
            <TextField
              select
              label="History"
              size="small"
              sx={{
                minWidth: 220
              }}
              value={activeMetric}
              onChange={(e) =>
                update({
                  metric: e.target.value
                })
              }
            >
              {conditional && <MenuItem value="contribution">Integral contributions</MenuItem>}
              <MenuItem value="allocation">Sampling allocation</MenuItem>
              <MenuItem value="hit_rate">Hit rate</MenuItem>
            </TextField>
            <TextField
              select
              label="Horizontal axis"
              size="small"
              sx={{
                minWidth: 210
              }}
              value={axis}
              onChange={(e) =>
                update({
                  axis: e.target.value,
                  zoom: FULL_ZOOM,
                  yZoom: FULL_ZOOM
                })
              }
            >
              <MenuItem value="samples">Completed samples</MenuItem>
              <MenuItem value="wall" disabled={!state.frames.some((f) => finite(f.wall_time_ms))}>
                Elapsed time
              </MenuItem>
              <MenuItem
                value="sampler"
                disabled={!state.frames.some((f) => finite(f.sampler_uptime_ms))}
              >
                Sampler time (reported)
              </MenuItem>
            </TextField>
            <FormControlLabel
              control={
                <Checkbox
                  checked={value?.follow !== false}
                  onChange={(e) =>
                    update({
                      follow: e.target.checked,
                      zoom: {
                        start: value?.zoom?.start ?? 0,
                        end: 100
                      }
                    })
                  }
                />
              }
              label="Follow latest"
            />

            {activeMetric === 'contribution' ? (
              <FormControlLabel
                control={
                  <Checkbox
                    checked={value?.total !== false}
                    onChange={(e) =>
                      update({
                        total: e.target.checked
                      })
                    }
                  />
                }
                label="Show total"
              />
            ) : (
              <FormControlLabel
                control={
                  <Checkbox
                    checked={interval}
                    onChange={(e) =>
                      update({
                        interval: e.target.checked
                      })
                    }
                  />
                }
                label="Between snapshots"
              />
            )}

          </ControlBar>
          <Typography variant="body2" color="text.secondary">
            {conditional
              ? 'Conditional integral contributions; bands show ±1 reported standard error. Legend selections affect visibility only.'
              : 'Pooled training statistics. Stratum plots show allocation and matches; stratified estimates are not used during training.'}
          </Typography>
          {conditional && total && (
            <EstimateValueBlock label={`Total ${state.components[component]}`} value={{ kind: "estimate", value: total.mean,
              error: total.error, estimator: "stratified", basic: latest.basic?.[component], sample_count: latest.samples }} />
          )}
          <LazyChart
            ref={ref}
            option={option}
            style={{
              height: 330
            }}
            onEvents={navigationEvents(value, (patch) =>
              update({
                ...patch,
                ...(patch.zoom
                  ? {
                      follow: patch.zoom.end >= 99.5
                    }
                  : {})
              })
            )}
            notMerge
          />
          {axis === 'sampler' && (
            <Typography variant="caption">
              Times are the latest preceding performance reports for this task; missing timestamps
              are omitted.
            </Typography>
          )}
          <PercentRow
            label="Observed allocation (cumulative attempts / total)"
            rows={rows}
            values={rows.map((r) => fraction(r.attempts, latest.samples))}
          />
          <PercentRow
            label="Exact proposal probability"
            rows={rows}
            values={rows.map((r) => r.probability)}
          />
          <PercentRow
            label="Hit rate (matches / attempts)"
            rows={rows}
            values={rows.map((r) => fraction(r.matches, r.attempts))}
          />
          {conditional && (
            <PercentRow
              label={`Share of reported variance (${state.components[component]})`}
              rows={rows}
              values={indices.map((i) => varianceShares(latest.rows, component)[i])}
            />
          )}
          <Typography variant="caption" color="text.secondary">
            {conditional
              ? 'Variance shares require every stratum error to be available and nonzero total variance. Rare large contributions can make sampled errors optimistic.'
              : 'Exact probabilities are not queried for pooled training. Between-snapshot histories use differences of retained cumulative counts.'}
          </Typography>
        </Stack>
      </CardContent>
    </Card>
  );
}
