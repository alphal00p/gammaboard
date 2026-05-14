# Process API Examples

Gammaboard runs external evaluators and samplers as ordinary child processes.
The process can be Python, Rust, C++, or anything else that can read stdin and
write stdout.

## Protocol

The process speaks `gammaboard-jsonrpc-v1`: JSON-RPC messages framed with
`Content-Length` headers over stdin/stdout.

- stdin receives requests from Gammaboard.
- stdout is reserved for framed protocol responses.
- stderr is for logs and ordinary debug output.
- `args = { ... }` from the run TOML is passed unchanged in the `initialize`
  request.

The process command is explicit in run config:

```toml
kind = "process_evaluator"
command = ["nix", "shell", "path:./process_api/examples/python_scalar_sin#runtime", "-c", "gammaboard-example-evaluator-worker"]
args = { module = "demo_integrand", class = "SinIntegrand" }
```

Gammaboard does not append worker scripts or assume Python. The command must
start the process that speaks the protocol.

Evaluator workers receive `initialize` once, then batched `eval_batch`
requests. `eval_batch` returns `values_row_major`; for the current
single-component accumulator path this is one `f64` per sample. The protocol
already names observable `components` so vector-valued evaluators can use the
same row-major response shape once vector accumulators are enabled.

## Python Wrapper Pattern

The examples include small Python worker scripts:

- `examples/python_scalar_sin/evaluator_worker.py`
- `examples/python_sampler_symbolica_havana/sampler_worker.py`

These scripts implement the process protocol and then load a normal Python
class from `args.module` and `args.class`. This is only a convenience layer. A
non-Python runtime can implement the same protocol directly and ignore the
Python wrapper shape.

The Python implementation class is intentionally simple:

- evaluators implement `eval(xs_discrete, xs_continuous)` and return one value
  per sample for the default single-component observable
- samplers implement `sample_plan`, `produce_latent_batch`, training hooks, and
  optional `pdf`
- optional `from_config(...)` and `from_snapshot(...)` constructors receive the
  run dimensions and the TOML `args`

## Examples

`examples/python_scalar_sin` packages a NumPy scalar evaluator.

`examples/python_sampler_symbolica_havana` packages a Symbolica-backed Havana
sampler, including the pinned Symbolica Python wheel. It does not rely on a
host `symbolica` installation.

`examples/rust_breit_wigner_evaluator` packages a non-Python scalar evaluator
with Apptainer. It is the simplest reference for implementing the protocol
directly in another language.
