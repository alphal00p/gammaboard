# Rust Breit-Wigner Evaluator

Standalone non-Python process evaluator example packaged with Apptainer.

It demonstrates the direct process protocol shape:

- `src/protocol.rs` reads and writes `Content-Length` framed JSON-RPC.
- `src/evaluator.rs` owns config parsing and the vectorized function.
- `src/main.rs` only connects the protocol to the evaluator.

Build the worker image from this directory:

```bash
apptainer build runtime.sif apptainer.def
```

Run config can then start it with:

```toml
command = ["apptainer", "exec", "process_api/examples/rust_breit_wigner_evaluator/runtime.sif", "breit-wigner-worker"]
```

The evaluator expects two continuous coordinates and no discrete axes. Channels
are configured as a resonance mixture in `args`:

```toml
continuous_dims = 2
discrete_cardinalities = []
args = { masses = [0.25, 0.5, 0.75], widths = [0.04, 0.06, 0.05], channel_weights = [1.0, 0.7, 1.3] }
```

The implemented function is:

```text
sum_i channel_weight[i] * exp(-y) / ((x - mass[i])^2 + width[i]^2)
```
