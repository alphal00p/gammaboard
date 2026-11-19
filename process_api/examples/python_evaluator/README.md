# NumPy evaluator tutorial

`SinIntegrand` returns unweighted scalar values on a mixed domain: two discrete
axes of cardinalities 2 and 3, and two continuous coordinates. Its run card uses
uniform sampling and shows discrete projections. It works in every GammaBoard
build; install the SDK using the [shared setup](../README.md).

Set the [run card](run.toml)'s `python` replacement to the worker interpreter, then
create it from the repository root:

```bash
./gammaboard run create process_api/examples/python_evaluator/run.toml
```

`src/run_evaluator.py` imports the class and calls
`gammaboard_process.run_evaluator(SinIntegrand)`. The class defines `eval`; the SDK
handles transport and array conversion. GammaBoard applies integration weights
and reduces the results.

The evaluator also demonstrates structured log levels, including one deliberately
emitted warning/error message. See [logging](../../protocol.md#notifications-logs-and-shutdown)
for filtering and stdout redirection.
