import { formatScientific } from '../../utils/formatters';
import { escapeXml } from './FigureExportActions';
import { formatAxisValue, gridColor } from './chartPrimitives';
import { finite, prefixLabel, stratumColor } from './strataUtils';
import { COVERAGE } from './pdfDiagnostics';
import { navigationZoom } from './DiagnosticControls';
export const DENSITY_COLORS = ['#f4f8fb', '#b8d9e9', '#5cabc9', '#146b99', '#062c54'];
const MISMATCH_COLORS = ['#0072b2', '#a9d1df', '#f5f5f5', '#e6b49c', '#b34016'];
export const lineComparisonOption = (
  c,
  { view, metric, mode, log, boundaries, prefixes, value }
) => {
  const density = view === 'density',
    categorical = view === 'coverage' || view === 'membership';
  const targetLog = density && log;
  const data = (values) =>
    values.map((v, i) => [c.axes.x[i], finite(v) && (!targetLog || v > 0) ? v : null]);
  const boundaryLines =
    boundaries && c.boundaries.length <= 64
      ? {
          symbol: 'none',
          silent: true,
          label: {
            show: false
          },
          lineStyle: {
            color: '#8a8f98',
            width: 1,
            type: 'dotted'
          },
          data: c.boundaries.map((x) => ({
            xAxis: x
          }))
        }
      : undefined;
  let series = [];
  if (density) {
    const color = c.k == null ? '#525b68' : stratumColor(c.k);
    const targetColor = c.k == null ? '#009e73' : stratumColor(c.k);
    const target = {
      name: 'Target',
      type: 'line',
      data: data(c.target),
      showSymbol: false,
      lineStyle: {
        type: 'dashed',
        width: 2,
        color: targetColor
      },
      itemStyle: {
        color: targetColor
      },
      yAxisIndex: ['shape', 'reference'].includes(mode) ? 0 : 1
    };
    if (c.k == null && prefixes.length > 0 && prefixes.length <= 8) {
      series = prefixes.map((p, k) => ({
        ...target,
        name: `Target ${prefixLabel(p)}`,
        data: data(c.target.map((v, i) => (c.actual[i] === k ? v : null))),
        lineStyle: {
          ...target.lineStyle,
          color: stratumColor(k)
        },
        itemStyle: {
          color: stratumColor(k)
        }
      }));
    } else series = [target];
    series.push({
      name: c.k == null && prefixes.length ? 'Sampler mixture' : 'Sampler PDF',
      type: 'line',
      data: c.stepPoints
        ? c.stepPoints.map(([x, v]) => [x, finite(v) && (!log || v > 0) ? v : null])
        : data(c.pdf),
      showSymbol: false,
      connectNulls: false,
      itemStyle: {
        color
      },
      lineStyle: {
        width: 2
      },
      markLine: boundaryLines
    });
  } else if (categorical) {
    const membership = view === 'membership';
    const groups = membership
      ? c.k == null
        ? prefixes.map((p, k) => ({
            label: prefixLabel(p),
            color: stratumColor(k),
            id: k
          }))
        : [
            {
              label: prefixLabel(prefixes[c.k]),
              color: stratumColor(c.k),
              id: c.k
            },
            {
              label: 'Other strata',
              color: '#a7afb9',
              id: -1
            }
          ]
      : COVERAGE;
    series = groups.map((g) => ({
      name: g.label,
      type: 'scatter',
      symbol: 'rect',
      symbolSize: 6,
      itemStyle: {
        color: g.color
      },
      data: c.axes.x.flatMap((x, i) =>
        (
          membership
            ? g.id === -1
              ? c.actual[i] != null && c.actual[i] !== c.k
              : c.actual[i] === g.id
            : c.status[i] === g.id
        )
          ? [[x, 1, c.actual[i]]]
          : []
      )
    }));
  } else {
    series = [
      {
        name: metric === 'log_ratio' ? 'log₁₀(PDF / target)' : 'PDF / target − 1',
        type: 'line',
        showSymbol: false,
        connectNulls: false,
        data: data(metric === 'log_ratio' ? c.logRatio : c.relative),
        itemStyle: {
          color: c.k == null ? '#0072b2' : stratumColor(c.k)
        },
        markLine: {
          ...boundaryLines,
          symbol: 'none',
          silent: true,
          data: [
            ...(boundaryLines?.data || []),
            {
              yAxis: 0,
              label: {
                formatter: 'Match'
              }
            }
          ]
        }
      }
    ];
  }
  const raw = density && !['shape', 'reference'].includes(mode);
  return {
    animation: false,
    grid: {
      left: 78,
      right: raw ? 80 : 46,
      top: 48,
      bottom: 72
    },
    legend: {
      top: 0,
      bottom: 'auto',
      type: 'scroll',
      show: !categorical || series.length <= 8
    },
    tooltip: {
      trigger: categorical ? 'item' : 'axis',
      valueFormatter: (v) => (finite(v) ? formatScientific(v, 6) : 'unavailable')
    },
    xAxis: {
      type: 'value',
      splitNumber: 3,
      name: 'Slice parameter',
      nameLocation: 'middle',
      nameGap: 30,
      axisLabel: {
        hideOverlap: true,
        formatter: formatAxisValue
      },
      splitLine: {
        lineStyle: {
          color: gridColor
        }
      }
    },
    yAxis: [
      {
        type: targetLog ? 'log' : 'value',
        name: categorical
          ? view === 'coverage'
            ? 'Coverage'
            : 'Stratum'
          : density
            ? mode === 'shape'
              ? 'Slice density'
              : 'Density'
            : 'Mismatch',
        min: categorical ? 0 : density && !log ? 0 : undefined,
        max: categorical ? 2 : undefined,
        axisLabel: {
          show: !categorical,
          formatter: formatAxisValue
        }
      },
      ...(raw
        ? [
            {
              type: targetLog ? 'log' : 'value',
              name: 'Importance target',
              axisLabel: {
                hideOverlap: true,
                formatter: formatAxisValue
              }
            }
          ]
        : [])
    ],
    dataZoom: navigationZoom(value),
    series
  };
};
export const planeComparisonOption = (
  c,
  values,
  {
    label,
    limits,
    log = false,
    categories = null,
    value,
    boundaries = false,
    mismatch = false,
    prefixes = []
  }
) => {
  const axes = c.axes;
  const categorical = categories != null;
  const points = values.flatMap((v, i) =>
    finite(v) && (categorical || !log || v > 0)
      ? [
          [
            i % axes.x.length,
            Math.floor(i / axes.x.length),
            !categorical && log ? Math.log10(v) : v,
            v
          ]
        ]
      : []
  );
  const colors = mismatch ? MISMATCH_COLORS : DENSITY_COLORS;
  const border = [];
  if (boundaries && c.actual.length)
    c.actual.forEach((v, i) => {
      if (
        v != null &&
        ((i % axes.x.length > 0 && c.actual[i - 1] != null && c.actual[i - 1] !== v) ||
          (i >= axes.x.length &&
            c.actual[i - axes.x.length] != null &&
            c.actual[i - axes.x.length] !== v))
      )
        border.push([i % axes.x.length, Math.floor(i / axes.x.length)]);
    });
  return {
    animation: false,
    grid: {
      left: 70,
      right: 50,
      top: 22,
      bottom: 110
    },
    tooltip: {
      formatter: (p) => {
        const [ix, iy, , raw] = p.value;
        return `u=${formatScientific(axes.x[ix], 4)}, v=${formatScientific(axes.y[iy], 4)}<br/>${categorical ? escapeXml(categories[raw]?.label ?? 'Unavailable') : `${escapeXml(label)}: ${formatScientific(raw, 6)}`}${prefixes.length && c.actual[iy * axes.x.length + ix] != null ? `<br/>Actual stratum: ${prefixLabel(prefixes[c.actual[iy * axes.x.length + ix]])}` : ''}`;
      }
    },
    xAxis: {
      type: 'category',
      data: axes.x,
      name: 'u',
      nameLocation: 'middle',
      nameGap: 26,
      axisLabel: {
        hideOverlap: true,
        formatter: formatAxisValue,
        interval: (i) => i === axes.x.length - 1 || i % Math.ceil(axes.x.length / 4) === 0
      }
    },
    yAxis: {
      type: 'category',
      data: axes.y,
      name: 'v',
      axisLabel: {
        hideOverlap: true,
        formatter: formatAxisValue,
        interval: (i) => i === axes.y.length - 1 || i % Math.ceil(axes.y.length / 4) === 0
      }
    },
    visualMap: categorical
      ? {
          type: 'piecewise',
          show: categories.length <= 8,
          seriesIndex: 0,
          orient: 'horizontal',
          left: 'center',
          bottom: 34,
          pieces: categories.map((c, k) => ({
            value: k,
            label: c.label,
            color: c.color
          }))
        }
      : {
          seriesIndex: 0,
          min: limits.min,
          max: limits.max,
          orient: 'horizontal',
          left: 'center',
          bottom: 34,
          inRange: {
            color: colors
          },
          text: [
            formatScientific(log ? 10 ** limits.max : limits.max, 3),
            formatScientific(log ? 10 ** limits.min : limits.min, 3)
          ]
        },
    dataZoom: navigationZoom(value),
    series: [
      {
        name: label,
        type: 'heatmap',
        data: points,
        progressive: 0
      },
      ...(border.length
        ? [
            {
              name: 'Stratum boundary',
              type: 'scatter',
              data: border,
              symbolSize: 2,
              itemStyle: {
                color: '#525b68'
              },
              silent: true,
              tooltip: {
                show: false
              }
            }
          ]
        : [])
    ]
  };
};
