import { expect, test } from 'vitest';
import {
  coverageStatus,
  diagnosticHistograms,
  pdfComparison,
  sharedColorLimits,
  sliceIntegral
} from './pdfDiagnostics';
import { scopeIndices } from './DiagnosticControls';
import { DENSITY_COLORS, planeComparisonOption, stratumDensityColor } from './pdfPlotOptions';
import { stratumColor } from './strataUtils';
import { echarts } from '../../lib/echarts';
import { figureCsv } from './FigureExportActions';
// SVG rendering needs text metrics, but jsdom does not provide a canvas context.
echarts.setPlatformAPI({ measureText: (text) => ({ width: String(text).length * 7 }) });
const ordinary = {
  geometry: {
    kind: 'line',
    geometry: {
      linspace: {
        start: 0,
        stop: 1,
        count: 3
      }
    }
  },
  data: {
    integrand_values: [1, 2, 1],
    pdf_values: [2, 1, 1],
    strata: null,
    pdf_steps: {
      edges: [0, 0.2, 1],
      densities: [[2, 0.75]]
    },
    global_abs_integrand_norm: 3
  }
};
test('ordinary and stratified comparisons share normalization; native bins override coarse quadrature', () => {
  const c = pdfComparison(ordinary);
  expect(c.pdfIntegral).toBe(1);
  expect(c.gridPdfIntegral).toBe(1.25);
  expect(c.stepPoints).toEqual([
    [0, 2],
    [0.2, 2],
    [0.2, 0.75],
    [1, 0.75]
  ]);
  expect(c.target).toEqual([2 / 3, 4 / 3, 2 / 3]);
  const ref = pdfComparison(ordinary, 'overall', 'reference');
  expect(ref.target).toEqual([1 / 3, 2 / 3, 1 / 3]);
  expect(ref.pdf).toEqual(ordinary.data.pdf_values);
});
test('coverage distinguishes all zero and missing cases, without finite fake ratios', () => {
  expect(
    [
      [1, 1],
      [0, 1],
      [1, 0],
      [0, 0],
      [null, 1]
    ].map(([p, t]) => coverageStatus(p, t))
  ).toEqual(['covered', 'uncovered', 'leakage', 'both_zero', 'missing']);
  const state = structuredClone(ordinary);
  state.data.pdf_steps = null;
  state.data.integrand_values = [1, 0, 0];
  state.data.pdf_values = [0, 1, 0];
  const c = pdfComparison(state);
  expect(c.relative).toEqual([-1, null, null]);
  expect(c.logRatio).toEqual([null, null, null]);
  expect(c.counts).toMatchObject({
    uncovered: 1,
    leakage: 1,
    both_zero: 1
  });
});
test('logarithmic heatmaps use exactly the same full limits for comparable data', () => {
  const limits = sharedColorLimits(
    [
      [0, 0.001, 10],
      [1, 100]
    ],
    {
      log: true
    }
  );
  expect([limits.min, limits.max]).toEqual([-3, 2]);
  const c = {
    axes: {
      x: [0, 1],
      y: [0, 1]
    },
    actual: []
  };
  const a = planeComparisonOption(c, [0.001, 1, 2, 10], {
    limits,
    log: true,
    label: 'Target'
  });
  const b = planeComparisonOption(c, [1, 2, 10, 100], {
    limits,
    log: true,
    label: 'PDF'
  });
  expect([a.visualMap.min, a.visualMap.max]).toEqual([b.visualMap.min, b.visualMap.max]);
});
test('rendered logarithmic heatmaps colour by the transformed value and keep raw tooltip data', () => {
  const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width: 640, height: 480 });
  try {
    const option = planeComparisonOption({ axes: { x: [0, 1], y: [0, 1] }, actual: [1, 0, 1, 0] },
      [0.001, 0.1, 10, 100], { label: 'PDF', limits: { min: -3, max: 2 }, log: true });
    chart.setOption(option);
    const data = chart.getModel().getSeriesByIndex(0).getData();
    expect(data.getItemGraphicEl(0).incremental).toBeFalsy();
    const first = data.getItemGraphicEl(0).shape;
    const adjacent = data.getItemGraphicEl(1).shape;
    expect(first.x + first.width).toBe(adjacent.x);
    const fill = (i) => echarts.color.parse(data.getItemVisual(i, 'style').fill);
    expect(fill(0)).toEqual(echarts.color.parse(DENSITY_COLORS[0]));
    expect(fill(3)).toEqual(echarts.color.parse(DENSITY_COLORS.at(-1)));
    expect(fill(1)).not.toEqual(fill(2));
    expect(option.tooltip.formatter({ value: data.getRawDataItem(0) })).toContain('1.000000e-3');
  } finally { chart.dispose(); }
});
test('stratum hue encodes actual membership while density controls saturation on both scales', () => {
  const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width: 640, height: 480 });
  try {
    const c = { axes: { x: [0, 1], y: [0, 1] }, actual: [0, 1, 0, null], k: 0 };
    const linear = planeComparisonOption(c, [100, 100, 1, 50], {
      label: 'PDF', limits: { min: 0, max: 100 }, stratumColours: true, prefixes: [[0], [1]]
    });
    chart.setOption(linear);
    const data = chart.getModel().getSeriesByIndex(0).getData();
    const fill = (i) => echarts.color.parse(data.getItemVisual(i, 'style').fill);
    expect(fill(0)).toEqual(echarts.color.parse(stratumColor(0)));
    expect(fill(1)).toEqual(echarts.color.parse(stratumColor(1))); // Leakage retains the actual hue.
    expect(fill(2)[0]).toBeGreaterThan(fill(0)[0]); // Low density fades toward white.
    expect(fill(3)).toEqual(echarts.color.parse(stratumDensityColor(null, 50, { min: 0, max: 100 })));
    const logarithmic = planeComparisonOption(c, [100, 100, 1, 50], {
      label: 'PDF', limits: { min: -2, max: 2 }, log: true, stratumColours: true
    });
    chart.setOption(logarithmic, true);
    const logData = chart.getModel().getSeriesByIndex(0).getData();
    expect(echarts.color.parse(logData.getItemVisual(2, 'style').fill)).toEqual(
      echarts.color.parse(stratumDensityColor(0, 0, { min: -2, max: 2 }))
    );
  } finally { chart.dispose(); }
});
test('histograms use shared bin edges and deterministic frequencies without MC errors', () => {
  const [a, b] = diagnosticHistograms(
    [
      [0, 1, 1, 2],
      [1, 2, 3]
    ],
    3
  );
  expect(a.map((x) => [x.start, x.stop])).toEqual(b.map((x) => [x.start, x.stop]));
  expect(a.every((x) => x.error === null)).toBe(true);
  expect(a.reduce((sum, x) => sum + x.value * (x.stop - x.start), 0)).toBeCloseTo(1);
});
test('comparisons paginate rather than dropping extra strata or renormalizing them', () => {
  expect(scopeIndices('compare', 11, 0)).toEqual([0, 1, 2, 3]);
  expect(scopeIndices('compare', 11, 2)).toEqual([8, 9, 10]);
  expect(scopeIndices('10', 11)).toEqual([10]);
  expect(scopeIndices('overall', 11)).toEqual([]);
  expect(scopeIndices('compare', 11, 99)).toEqual([8, 9, 10]);
  expect(scopeIndices('compare', 11, -1)).toEqual([0, 1, 2, 3]);
  expect(scopeIndices('compare', 11, 1.5)).toEqual([4, 5, 6, 7]);
});

test('log ratios stay finite when the relative ratio overflows', () => {
  const state = {
    geometry: { kind: 'line', geometry: { linspace: { start: 0, stop: 1, count: 2 } } },
    data: { integrand_values: [1e-320, 1], pdf_values: [1, 1], strata: null }
  };
  const c = pdfComparison(state);
  expect(c.relative[0]).toBeNull();
  expect(c.logRatio[0]).toBeGreaterThan(319);
  expect(Number.isFinite(c.logRatio[0])).toBe(true);
});

test('slice quadrature supports reversed axes and leaves incomplete or zero targets unavailable', () => {
  expect(sliceIntegral([0, 1, 2], [0, 1, 2])).toBe(2);
  expect(sliceIntegral([1, 1, 1, 1], [0, 2], [0, 3])).toBe(6);
  const state = structuredClone(ordinary);
  state.geometry.geometry.linspace = { start: 2, stop: 0, count: 3 };
  state.data.integrand_values = [0, 1, 2];
  expect(pdfComparison(state).target).toEqual([0, 0.5, 1]);
  for (const values of [
    [0, null, 2],
    [0, 0, 0]
  ]) {
    state.data.integrand_values = values;
    expect(pdfComparison(state).target).toEqual([null, null, null]);
  }
});

const state = {
  kind: 'pdf_comparison',
  geometry: { kind: 'line', geometry: { linspace: { start: 0, stop: 1, count: 3 } } },
  data: {
    integrand_values: [-2, 0, 2],
    pdf_values: [1, 1, 1],
    strata: {
      prefixes: [[0], [1, 3]],
      probabilities: [0.25, 0.75],
      actual_strata: [0, 0, 1],
      pdf_values: [
        [0.5, 0.2, 0.05],
        [0.5, 0.8, 0.95]
      ]
    }
  }
};

test('conditional PDFs divide only by exact prefix mass, keep leakage and gate only the target', () => {
  const c = pdfComparison(state, '0', 'conditional');
  expect(c.pdf).toEqual([2, 0.8, 0.2]);
  expect(c.target).toEqual([2, 0, 0]);
  expect(pdfComparison(state, 'overall', 'joint').pdf).toEqual([1, 1, 1]);
  expect(pdfComparison(state, '0', 'shape').pdf[2]).toBeGreaterThan(0);
  const unknown = structuredClone(state);
  unknown.data.strata.probabilities[0] = null;
  expect(pdfComparison(unknown, '0', 'conditional').pdf).toEqual([null, null, null]);
  expect(pdfComparison(unknown, '0', 'joint').pdf).toEqual([0.5, 0.2, 0.05]);
  expect(figureCsv(unknown)).toContain('"[1,3]"');
  expect(figureCsv(unknown)).not.toContain('Infinity');
});
