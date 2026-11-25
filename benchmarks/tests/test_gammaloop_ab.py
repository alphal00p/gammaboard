import json
from pathlib import Path
import tempfile
import tomllib
import unittest

from benchmarks.gammaloop_ab import card, scientific_result, summarize


class GammaLoopABTests(unittest.TestCase):
    def test_scientific_check_requires_full_histograms(self):
        output = dict(bundle=dict(histograms={}), estimate=dict(components=[
            dict(name='real', state=dict(count=2, sum_weighted_value=6., sum_sq=20.))
        ]))
        results = dict(tasks=[dict(name='train', snapshot=dict(persisted_output=output))])
        with self.assertRaises(RuntimeError):
            scientific_result(results, 'train', 2)
        output['bundle']['histograms']['integral'] = dict(sample_count=2)
        result = scientific_result(results, 'train', 2)
        self.assertEqual(result['integral']['real'], dict(mean=3., stderr=1.))
        self.assertEqual(result['histogram_count'], 1)

    def test_cards_have_identical_work_and_observables(self):
        params = dict(state_folder='/state with spaces', integrand_name='1L', training_projection='abs')
        domain = dict(rectangular=dict(continuous_dims=3, discrete_cardinalities=[]))
        counts = dict(train=8192, inference=16384)
        a, b = [tomllib.loads(card(mode, params, domain, Path('/worker'), counts, 512))
                for mode in ('native', 'process')]
        self.assertEqual(a['task_queue'], b['task_queue'])
        self.assertEqual(b['evaluator']['args'], params)
        self.assertNotIn('output', b['evaluator'])
        self.assertNotIn('domain', b['evaluator'])
        self.assertEqual(b['task_queue'][1]['sampler']['from_stage'], 'train')
        for task in b['task_queue']:
            self.assertEqual(task['accumulator'], dict(config='gammaloop'))
        for key in ('evaluator_requirements', 'sampler_requirements', 'sampler_aggregator_runner_params'):
            self.assertEqual(a[key], b[key])

    def test_summary_compares_matched_work_and_reports_variability(self):
        rows = [dict(case=str(i), backend=backend, phase=phase, samples=600,
                     seconds=seconds, samples_per_second=600/seconds, reported_cpu_seconds=0)
                for phase in ('train', 'inference')
                for backend, durations in [('native', [28, 32]), ('process', [40, 44])]
                for i, seconds in enumerate(durations)]
        with tempfile.TemporaryDirectory() as directory:
            summary = summarize(Path(directory), rows, {})
            process = next(r for r in summary if r['backend'] == 'process')
            self.assertEqual(process['mean_seconds'], 42)
            self.assertAlmostEqual(process['time_ratio_to_native'], 1.4)
            self.assertEqual(process['min_seconds'], 40)
            self.assertEqual(process['max_seconds'], 44)
            self.assertEqual(len(json.loads(Path(directory, 'summary.json').read_text())['measurements']), 8)


if __name__ == '__main__':
    unittest.main()
