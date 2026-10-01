"""CLI planning, failure reporting and private deployment cleanup."""

import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch
from benchmarks import cli as benchmark
from benchmarks import common as bench


class SuiteTests(unittest.TestCase):
    def test_frontier_plan_is_sparse_and_feedback_modes_are_paired(self):
        args = benchmark.parser().parse_args(["plan", "--workers", "1", "16", "--cpu-limit", "8"])
        suite = benchmark.frontier_suite(args)
        self.assertEqual(suite["workers"], [1, 16])
        self.assertEqual(suite["modes"], ["materialized", "training"])
        self.assertLess(suite["infrastructure_cores"], 8)

    def session(self, root):
        session = bench.Session.__new__(bench.Session)
        session.directory = root / "private"
        session.directory.mkdir()
        session.output = root
        session.workers = []
        session.cli = Mock()
        session.db_command = Mock()
        return session

    def test_cleanup_success_removes_only_private_database_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            session = self.session(root)
            session.__exit__(None)
            self.assertFalse(session.directory.exists())
            self.assertTrue(root.exists())
            session.db_command.assert_called_once_with("stop")
            self.assertTrue(json.loads((root / "cleanup.json").read_text())["clean"])

    def test_database_cleanup_failure_is_visible_and_retains_diagnostics(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            session = self.session(root)
            session.db_command.side_effect = RuntimeError("stop failed")
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaisesRegex(
                RuntimeError, "stop failed"
            ):
                session.__exit__(None)
            self.assertTrue(session.directory.exists())
            self.assertFalse(json.loads((root / "cleanup.json").read_text())["clean"])

    def test_cleanup_does_not_mask_existing_measurement_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            session = self.session(Path(tmp))
            session.db_command.side_effect = RuntimeError("stop failed")
            with contextlib.redirect_stderr(io.StringIO()):
                session.__exit__(RuntimeError)
            self.assertTrue(session.directory.exists())

    def test_all_families_share_an_immutable_input_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo = root / "repo"
            (repo / "benchmarks").mkdir(parents=True)
            (repo / "benchmarks" / "cli.py").write_text("original")
            (repo / "migrations").mkdir()
            (repo / "migrations" / "001.sql").write_text("migration")
            binary = root / "binary"
            binary.write_bytes(b"original executable")
            output = root / "suite"
            (output / "frontier").mkdir(parents=True)
            (output / "protocol").mkdir()
            bench.write_json(output / "manifest.json", dict(experiment="suite"))
            with patch.object(bench, "ROOT", repo), patch.dict(bench.os.environ):
                bench.os.environ.pop("GAMMABOARD_MIGRATIONS_DIR", None)
                first, hashes = bench.preserve_inputs(output / "frontier", binary)
                binary.write_bytes(b"later rebuild")
                second, later_hashes = bench.preserve_inputs(output / "protocol", binary)
                self.assertEqual(first, second)
                self.assertEqual(second.read_bytes(), b"original executable")
                self.assertEqual(hashes, later_hashes)
                self.assertEqual(
                    Path(bench.os.environ["GAMMABOARD_MIGRATIONS_DIR"]),
                    output / "inputs/migrations",
                )
                self.assertFalse((output / "protocol/inputs").exists())

    def test_partial_suite_report_keeps_failure_visible(self):
        from benchmarks.reporting import aggregate, write_html

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            child = root / "protocol"
            child.mkdir()
            bench.write_json(root / "manifest.json", dict(status="incomplete", elapsed_seconds=3))
            write_html(child / "report.html", "Protocol", "<h1>Protocol</h1><p>completed</p>")
            content = aggregate(root, [child, root / "missing"]).read_text()
            self.assertIn("incomplete", content)
            self.assertIn("protocol/report.html", content)
            self.assertNotIn("<h1>Protocol</h1>", content)


if __name__ == "__main__":
    unittest.main()
