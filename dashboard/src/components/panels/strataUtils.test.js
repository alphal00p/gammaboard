import { expect, test } from 'vitest';
import { stratumHistory, varianceShares } from './strataUtils';

test('variance shares use squared errors and never conceal missing or all-zero errors', () => {
  const rows = [{ estimates: [{ error: 3 }] }, { estimates: [{ error: 4 }] }];
  expect(varianceShares(rows, 0)).toEqual([0.36, 0.64]);
  rows[0].estimates[0].error = null;
  expect(varianceShares(rows, 0)).toEqual([null, null]);
  rows.forEach((r) => (r.estimates[0].error = 0));
  expect(varianceShares(rows, 0)).toEqual([null, null]);
});

test('training histories use count differences without inventing conditional estimates', () => {
  const frames = [
    { samples: 10, rows: [{ prefix: [1, 3], attempts: 4, matches: 2, estimates: [] }] },
    { samples: 20, rows: [{ prefix: [1, 3], attempts: 5, matches: 3, estimates: [] }] }
  ];
  expect(stratumHistory({ frames }, 0, 'allocation', true)[0].points.map((p) => p.y)).toEqual([
    0.4, 0.1
  ]);
  expect(stratumHistory({ frames }, 0, 'hit_rate', true)[0].points.map((p) => p.y)).toEqual([
    0.5, 1
  ]);
  expect(stratumHistory({ frames }, 0, 'contribution')[0].points.map((p) => p.y)).toEqual([
    null,
    null
  ]);
});
