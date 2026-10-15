import importlib.util
from pathlib import Path
import tempfile
import unittest

import tomlkit

spec = importlib.util.spec_from_file_location("convert_run", Path(__file__).parents[1] / "convert_run.py")
converter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(converter)


class ConversionTests(unittest.TestCase):
    def test_old_fields_and_opaque_args(self):
        original = '''name = "test"
[evaluator]
kind = "symbolica"
args = ["x"]
expr = "a*x"
constants = {a = 2}
[[task_queue]]
kind = "sample"
nr_samples = 10
sampler_aggregator = {config = {kind = "process_sampler", command = ["worker"], requires_training_values = true, args = {sampler_aggregator = "user-data"}}}
measurement = {metric = {name = "variance", component = "value"}}
'''
        result, changes = converter.convert(tomlkit.parse(original))
        self.assertEqual(result["evaluator"]["source"]["constants"]["a"], 2)
        task = result["task_queue"][0]
        self.assertEqual(task["stop_condition"]["max_samples"], 10)
        self.assertEqual(task["measurement"]["selection"]["name"], "variance")
        self.assertEqual(task["sampler"]["config"]["args"]["sampler_aggregator"], "user-data")
        rendered = tomlkit.dumps(result)
        self.assertEqual(tomlkit.dumps(converter.convert(tomlkit.parse(rendered))[0]), rendered)
        self.assertTrue(changes)

    def test_ambiguous_sources_are_not_guessed(self):
        for raw in [
            'kind="symbolica"\nexpr="x"\nsource={kind="expression",expr="y"}',
            'kind="sample"\nnr_samples=1\nstop_condition={max_samples=2}',
            'kind="sample"\nstop_condition={projection="imag",relative_error=0.1}',
        ]:
            with self.assertRaises(ValueError):
                converter.convert(tomlkit.parse(raw))

    def test_output_cannot_overwrite_input(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.toml"
            original = 'name="untouched"\n'
            path.write_text(original)
            with self.assertRaises(SystemExit) as error:
                converter.main([str(path), "--output", str(path)])
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(path.read_text(), original)
