# Rust Ragged-Domain Evaluator

Standalone non-Python process evaluator example packaged with Apptainer.

It demonstrates the direct process protocol shape:

- `src/protocol.rs` reads and writes `Content-Length` framed JSON-RPC.
- `src/evaluator.rs` owns config parsing and a vectorized ragged-domain function.
- `src/main.rs` only connects the protocol to the evaluator.

Build the worker image from this directory:

```bash
apptainer build runtime.sif apptainer.def
```

Run config can then start it with:

```toml
command = ["apptainer", "exec", "process_api/examples/rust_breit_wigner_evaluator/runtime.sif", "breit-wigner-worker"]
```

The evaluator consumes ragged row-major batches. The demo domain is:

```text
d0 = 0 -> 3 continuous dimensions
d0 = 1, d1 = 0 -> 1 continuous dimension
d0 = 1, d1 = 1, d2 in 0..=4 -> 5 continuous dimensions
```

Configure it through `evaluator.domain`; `args.scale` is optional:

```toml
args = { scale = 1.0 }
domain = { Discrete = { axis_label = "d0", branches = [
  { index = 0, domain = { Continuous = { dims = 3 } } },
  { index = 1, domain = { Discrete = { axis_label = "d1", branches = [
    { index = 0, domain = { Continuous = { dims = 1 } } },
    { index = 1, domain = { Discrete = { axis_label = "d2", branches = [
      { index = 0, domain = { Continuous = { dims = 5 } } },
      { index = 1, domain = { Continuous = { dims = 5 } } },
      { index = 2, domain = { Continuous = { dims = 5 } } },
      { index = 3, domain = { Continuous = { dims = 5 } } },
      { index = 4, domain = { Continuous = { dims = 5 } } },
    ] } } },
  ] } } },
] } }
```
