# Prepare the GammaLoop example state

From the repository root:

```bash
just prepare-gammaloop resources/states/showcase/gg_hhh
```

This runs the explicit Cargo helper target `prepare_gammaloop`, using GammaBoard's
pinned GammaLoop dependency. It requires the `gammaloop` build feature and refuses
to overwrite an existing output directory.

`generation.toml` is a **GammaLoop generation recipe**, embedded by `main.rs`.
The helper generates the native gg → HHH GL15 amplitude and observable definitions
used by the [four-phases run](../../resources/templates/runs/gammaloop-four-phases.toml).
It does not create GammaBoard database records, launch workers or start a server.

The state directory must be available under the configured resource roots of
every worker running that example. See the [setup guide](../../docs/quickstart.md#default-examples)
for the physics configuration and the illustrative target-uncertainty values.
