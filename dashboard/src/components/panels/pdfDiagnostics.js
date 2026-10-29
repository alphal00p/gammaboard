import { finite, fraction } from './strataUtils';
export const rasterAxes = ({ kind, geometry: g }) => {
  const axis = (s) =>
    Array.from(
      {
        length: s.count
      },
      (_, i) => (s.count === 1 ? s.start : s.start + ((s.stop - s.start) * i) / (s.count - 1))
    );
  return kind === 'line'
    ? {
        x: axis(g.linspace),
        y: null
      }
    : {
        x: axis(g.u_linspace),
        y: axis(g.v_linspace)
      };
};

// Trapezoid weights integrate over the displayed parameter slice. They do not
// marginalize hidden coordinates or estimate a full-domain normalization.
export const sliceIntegral = (values, x, y = null) => {
  if (
    x.length < 2 ||
    (y && y.length < 2) ||
    values.length !== x.length * (y?.length || 1) ||
    values.some((v) => !finite(v))
  )
    return null;
  const dx = Math.abs(x.at(-1) - x[0]) / (x.length - 1);
  const dy = y ? Math.abs(y.at(-1) - y[0]) / (y.length - 1) : 1;
  if (!(dx > 0) || !(dy > 0)) return null;
  return values.reduce((sum, v, i) => {
    const ix = i % x.length,
      iy = Math.floor(i / x.length);
    const wx = ix === 0 || ix === x.length - 1 ? 0.5 : 1;
    const wy = y && (iy === 0 || iy === y.length - 1) ? 0.5 : 1;
    return sum + v * wx * wy * dx * dy;
  }, 0);
};
export const sumDefined = (values) =>
  values.every(finite) ? values.reduce((a, b) => a + b, 0) : null;
const divide = (values, divisor) =>
  values.map((v) =>
    finite(v) && finite(divisor) && divisor > 0 && finite(v / divisor) ? v / divisor : null
  );
export const coverageStatus = (pdf, target) =>
  !finite(pdf) || !finite(target)
    ? 'missing'
    : pdf === 0 && target > 0
      ? 'uncovered'
      : pdf > 0 && target === 0
        ? 'leakage'
        : pdf === 0 && target === 0
          ? 'both_zero'
          : 'covered';
export const COVERAGE = [
  {
    id: 'covered',
    label: 'Both positive',
    color: '#009e73'
  },
  {
    id: 'uncovered',
    label: 'Positive target, zero PDF',
    color: '#d55e00'
  },
  {
    id: 'leakage',
    label: 'Zero target, positive PDF',
    color: '#0072b2'
  },
  {
    id: 'both_zero',
    label: 'Both zero',
    color: '#8a8f98'
  },
  {
    id: 'missing',
    label: 'Unavailable',
    color: '#cc79a7'
  }
];
export const pdfComparison = (state, scope = 'overall', mode = 'shape') => {
  const data = state.data,
    strata = data.strata,
    axes = rasterAxes(state.geometry);
  const k = scope === 'overall' || !strata ? null : Number(scope);
  const actual = strata?.actual_strata || [];
  const rawTarget = data.integrand_values.map((v, i) =>
    !finite(v) ? null : k == null || actual[i] === k ? Math.abs(v) : 0
  );
  const joint = k == null ? data.pdf_values : strata.pdf_values[k];
  const probability = k == null ? 1 : strata.probabilities[k];
  const steps = data.pdf_steps;
  const stepValues = steps
    ? k == null
      ? steps.edges.slice(1).map((_, i) => sumDefined(steps.densities.map((row) => row[i])))
      : steps.densities[k]
    : null;
  const stepIntegral =
    steps && stepValues
      ? sumDefined(
          stepValues.map((v, i) => (finite(v) ? v * (steps.edges[i + 1] - steps.edges[i]) : null))
        )
      : null;
  const gridPdfIntegral = sliceIntegral(joint, axes.x, axes.y);
  const pdfIntegral = steps ? stepIntegral : gridPdfIntegral;
  const targetIntegral = sliceIntegral(rawTarget, axes.x, axes.y);
  const pdfDivisor = mode === 'shape' ? pdfIntegral : mode === 'conditional' ? probability : 1;
  const targetDivisor =
    mode === 'shape' ? targetIntegral : mode === 'reference' ? data.global_abs_integrand_norm : 1;
  const pdf = divide(joint, pdfDivisor),
    target = divide(rawTarget, targetDivisor);
  const status = joint.map((p, i) => coverageStatus(p, rawTarget[i]));
  const ratio = pdf.map((p, i) =>
    finite(p) && finite(target[i]) && target[i] > 0 && finite(p / target[i]) ? p / target[i] : null
  );
  const relative = ratio.map((r) => (finite(r) ? r - 1 : null));
  const logRatio = pdf.map((p, i) =>
    finite(p) && p > 0 && finite(target[i]) && target[i] > 0
      ? Math.log10(p) - Math.log10(target[i])
      : null
  );
  const targetTotal = sliceIntegral(
    data.integrand_values.map((v) => (finite(v) ? Math.abs(v) : null)),
    axes.x,
    axes.y
  );
  const mixtureTotal = sliceIntegral(data.pdf_values, axes.x, axes.y);
  const maskedIntegral = (values, predicate) =>
    sliceIntegral(
      values.map((v, i) => (!finite(v) || actual[i] == null ? null : predicate(actual[i]) ? v : 0)),
      axes.x,
      axes.y
    );
  const leakage =
    k == null
      ? null
      : fraction(
          maskedIntegral(joint, (label) => label !== k),
          gridPdfIntegral
        );
  const actualMass =
    k == null
      ? null
      : fraction(
          maskedIntegral(data.pdf_values, (label) => label === k),
          mixtureTotal
        );
  const stepPoints = stepValues
    ? divide(stepValues, pdfDivisor).flatMap((v, i) => [
        [steps.edges[i], v],
        [steps.edges[i + 1], v]
      ])
    : null;
  const counts = Object.fromEntries(
    COVERAGE.map((c) => [c.id, status.filter((s) => s === c.id).length])
  );
  return {
    scope,
    k,
    axes,
    actual,
    rawTarget,
    joint,
    pdf,
    target,
    probability,
    targetIntegral,
    pdfIntegral,
    gridPdfIntegral,
    pdfIntegralMethod: steps ? 'Exact native bins' : 'Trapezoidal grid estimate',
    pdfDivisor,
    targetDivisor,
    targetShare: fraction(targetIntegral, targetTotal),
    actualMass,
    leakage,
    status,
    counts,
    relative,
    logRatio,
    stepPoints
  };
};
export const sharedColorLimits = (
  arrays,
  { log = false, symmetric = false, manual = null } = {}
) => {
  let min = Infinity,
    max = -Infinity;
  for (const values of arrays)
    for (const value of values) {
      if (!finite(value) || (log && value <= 0)) continue;
      const v = log ? Math.log10(value) : value;
      min = Math.min(min, v);
      max = Math.max(max, v);
    }
  if (!finite(min) || !finite(max)) {
    min = log ? -1 : 0;
    max = 1;
  }
  if (symmetric) {
    max = Math.max(Math.abs(min), Math.abs(max), 1e-12);
    min = -max;
  } else if (!log) min = Math.min(0, min);
  if (min === max) {
    const pad = Math.max(1, Math.abs(max) * 0.1);
    min -= pad;
    max += pad;
  }
  const auto = {
    zmin: min,
    zmax: max
  };
  if (manual && finite(manual.min) && finite(manual.max) && manual.min < manual.max) {
    min = manual.min;
    max = manual.max;
  }
  return {
    min,
    max,
    auto
  };
};
export const diagnosticHistograms = (arrays, bins = 31) => {
  const finiteArrays = arrays.map((a) => a.filter(finite));
  let lo = Infinity,
    hi = -Infinity;
  for (const a of finiteArrays)
    for (const v of a) {
      lo = Math.min(lo, v);
      hi = Math.max(hi, v);
    }
  if (!finite(lo) || !finite(hi)) return arrays.map(() => []);
  if (lo === hi) {
    const pad = Math.max(1, Math.abs(lo) * 0.1);
    lo -= pad / 2;
    hi += pad / 2;
  }
  const width = (hi - lo) / bins;
  return finiteArrays.map((a) => {
    if (!a.length) return [];
    const counts = Array(bins).fill(0);
    for (const v of a) counts[Math.min(bins - 1, Math.max(0, Math.floor((v - lo) / width)))] += 1;
    return counts.map((count, i) => ({
      start: lo + i * width,
      stop: lo + (i + 1) * width,
      value: count / (a.length * width),
      error: null
    }));
  });
};
