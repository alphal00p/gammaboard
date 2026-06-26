# Python Scalar Sin Evaluator

This example uses a plain Python virtual environment.

```bash
cd process_api/examples/python_scalar_sin
python -m venv .venv
. .venv/bin/activate
pip install -r requirements.txt
```

`requirements.txt` installs the GammaBoard Python process wrapper from the
repository, the same way an external user runtime can depend on it.

The worker entrypoint is:

```bash
.venv/bin/python -u -m run_evaluator
```

`src/run_evaluator.py` imports `SinIntegrand` and hands it to
`gammaboard_process.run_evaluator(...)`. Custom evaluators should follow the
same shape: implement the class, then call `run_evaluator(MyEvaluator)`.

`SinIntegrand` also demonstrates worker logging: a `print` (captured at info), a
config line via `gammaboard_process.log(..., level="info")`, a one-time showcase
of every level (trace/debug/info/warn/error) on the first batch, and per-batch
`debug` detail. `debug`/`trace` are dropped by default — set
`GAMMABOARD_LOG_LEVEL=debug` (worker) and `db_gammaboard_level = "debug"` (server)
to see them. See [../../../docs/process-runtime.md](../../../docs/process-runtime.md#logging).
