# Examples

Run and task cards are shared across local, ITPhlies, and UBELIX deployments.
Choose the worker-launch profile separately; there is no second set of physics
cards under the cluster operations directory.

| Purpose | Card or guide | Requirements |
| --- | --- | --- |
| Installation check | `resources/templates/runs/installation-smoke.toml` | Two local workers; exact unit integral, 10,000 samples |
| Adaptive sampling and images | `resources/templates/runs/ghost_bump.toml` | Symbolica; 200,000 training samples and two images |
| GammaLoop training and inference | `resources/templates/runs/gammaloop.toml` | Generated ttH LO state; 100,000 training and 1,000,000 inference samples |
| Multiple physics contributions | `resources/templates/runs/integration-campaign-qft-like.toml` | Same state; references `tt_h.toml` for graph-group children |
| MADNIS in Apptainer | `resources/templates/runs/ghost_bump_madnis_apptainer.toml` | GPU worker and `resources/runtimes/madnis/madnis.sif` |
| MADNIS in a local virtualenv | `integrations/madnis/README.md` | The integration's Python environment |
| External evaluator/sampler protocol | `process_api/README.md` | Dependencies documented in each example |
| Throughput and scaling | [benchmarking.md](benchmarking.md) | Prebuilt binary; no physics state or GPU needed |

Create a run with `gammaboard run create CARD`, start workers, and use
`gammaboard run resume RUN --max-evaluators N`. Pass the same runtime config and
port offset used by the deployment. The checkout's `./gammaboard` wrapper builds
the binary automatically; an installed binary uses the same CLI.

Sampling examples attach an accumulator directly to the sample task. Training
followed by inference uses `publish_result = false` on training and a fresh
inference accumulator, while reusing the trained sampler. This keeps adaptive
training samples out of the published inference estimate.

`resources/templates/tasks/train_sample.toml` appends the larger GammaLoop
training/inference sequence to an existing physics run. Its defaults are
100 million and 1 billion samples; reduce the replacement values for development.
Examples with generated states, process runtimes, or GPUs are not smoke tests.

Resource paths resolve against the configured resources root. Keep generated
states and writable checkpoints there; adjust process commands when moving from
a checkout virtualenv to an Apptainer runtime. Cluster sync copies templates,
not generated states or environments.
