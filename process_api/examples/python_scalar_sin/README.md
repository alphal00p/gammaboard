# Python Scalar Sin Evaluator

This example uses a plain Python virtual environment.

```bash
cd process_api/examples/python_scalar_sin
python -m venv .venv
. .venv/bin/activate
pip install -r requirements.txt --force-reinstall
```

`requirements.txt` installs the GammaBoard Python process wrapper from the
current checkout so its protocol matches the Rust binary. External runtimes should
install the SDK from the same GammaBoard revision.

The worker entrypoint is:

```bash
.venv/bin/python -u -m run_evaluator
```

`src/run_evaluator.py` imports `SinIntegrand` and hands it to
`gammaboard_process.run_evaluator(...)`. Custom evaluators should follow the
same shape: implement the class, then call `run_evaluator(MyEvaluator)`.

`SinIntegrand` demonstrates ordinary and structured logging. For log filtering,
see [logging and shutdown](../../README.md#logging-and-shutdown).
