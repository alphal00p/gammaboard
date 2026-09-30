import unittest

from benchmark_process import summarize


class ProcessSummaryTests(unittest.TestCase):
    def test_overhead_uses_paired_calls_and_excludes_callback_work(self):
        row = dict(operation='eval',batch=16,work=64,feedback=True,
                   wall_seconds=[.003,.010],callback_seconds=[.002,.008])
        result = summarize(dict(rows=[row]))[0]
        self.assertAlmostEqual(result['overhead_us'],1500)
        self.assertAlmostEqual(result['callback_us'],5000)
        self.assertAlmostEqual(result['overhead_fraction'],3/13)
        self.assertEqual(result['calls'],2)

    def test_missing_pairs_and_negative_residuals_are_rejected(self):
        for wall,callback in [([],[]),([1],[.5,.5]),([1],[2]), ([float("nan")],[0]), ([1],[float("nan")]), ([float("inf")],[0])]:
            with self.subTest(wall=wall,callback=callback),self.assertRaises(ValueError):
                summarize(dict(rows=[dict(wall_seconds=wall,callback_seconds=callback)]))


if __name__ == '__main__': unittest.main()
