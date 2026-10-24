import { fireEvent, render, screen } from '@testing-library/react';
import { expect, test, vi } from 'vitest';
import { EstimateValueBlock } from './BasicPanels';
import { formatF64Full, formatScientific } from '../../utils/formatters';
import {
  buildHistogramData,
  buildRelativeErrorStepData,
  normalizeGammaLoopHistogramBins
} from './histogramUtils';

vi.mock('../LatexFormula', () => ({ default: ({ latex }) => <span>{latex}</span> }));

test('expanded estimates pair the pooled error with its own mean', () => {
  render(
    <EstimateValueBlock
      value={{
        kind: 'estimate',
        value: 1.5,
        error: 0.5,
        estimator: 'stratified',
        basic: { mean: 1.8, error: 0.7 }
      }}
    />
  );
  expect(screen.getByText('1.5(0.5)\\;\\left(33.333\\%\\right)')).toBeInTheDocument();
  const summary = screen.getByText('Estimates and full precision (f64)');
  fireEvent.click(summary);
  expect(summary.closest('details')).toHaveAttribute('open');
  expect(screen.getByText(/strat_err =/)).toHaveTextContent(`mean = ${formatF64Full(1.5)}`);
  expect(screen.getByText(/strat_err =/)).toHaveTextContent(`basic_mean = ${formatF64Full(1.8)}`);
  expect(screen.getByText(/strat_err =/)).toHaveTextContent(`basic_err = ${formatF64Full(0.7)}`);
});

test('provisional means keep an unavailable error and pooled stages never imply stratified estimation', () => {
  const { rerender } = render(
    <EstimateValueBlock value={{ value: 1.5, error: null, estimator: 'pooled' }} />
  );
  expect(screen.getByText('pooled estimate; error unavailable')).toBeInTheDocument();
  expect(screen.getByText(/strat_err = not used/)).toHaveTextContent('basic_err = unavailable');
  expect(screen.queryByText(/basic_err = 0/)).not.toBeInTheDocument();
  rerender(
    <EstimateValueBlock
      value={{
        value: null,
        error: null,
        estimator: 'stratified',
        basic: { mean: 1.5, error: null }
      }}
    />
  );
  expect(screen.getByText('Mean unavailable')).toBeInTheDocument();
  expect(screen.getByText(/strat_err =/)).toHaveTextContent('insufficient stratum coverage');
  expect(formatF64Full(null)).toBe('n/a');
  expect(formatScientific(null)).toBe('n/a');
});

test('a one-attempt histogram preserves an unknown error through plotting and relative errors', () => {
  const bins = normalizeGammaLoopHistogramBins({
    sample_count: 1,
    bins: [{ x_min: 0, x_max: 1, sum_weights: 2, sum_weights_squared: 4 }]
  });
  expect(bins[0].value).toBe(2);
  expect(bins[0].error).toBeNull();
  const plotted = buildHistogramData(bins);
  expect(plotted[0].error).toBeNull();
  expect(buildRelativeErrorStepData(plotted)[0].relative_error).toBeNull();
});
