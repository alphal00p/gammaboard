# NumPy sampler tutorial

`UniformSampler` draws independent uniform points on a rectangular domain:
continuous coordinates in `[0,1]`, plus uniform discrete choices. It needs only
NumPy and the SDK; use the [shared setup](../README.md).

The example covers bounded generation, FIFO feedback, `WAITING`/`FINISHED`,
JSON checkpoints with pending draws, exact recovery, fresh stage initialization,
joint PDFs and discrete masses. It works with leading stratum dimensions without
interpreting their labels. Its full integration weight is the product of all
discrete cardinalities, the reciprocal of its joint PDF.

With `training_samples > 0`, it requests feedback for that many draws and limits
outstanding samples using `feedback_window`. This demonstrates the training
protocol; **it does not adapt the uniform proposal**. Inference uses
`training_samples = 0` and runs until GammaBoard's task budget is reached.
Stage handoff starts a fresh stream using the destination seed; checkpoint recovery
restores the exact stream and pending-feedback queue.

The [run card](run.toml) integrates the unit function in two stages. Set its
`python` replacement to an interpreter with the SDK, then create it from the
repository root:

```bash
./gammaboard run create process_api/examples/python_sampler/run.toml
```

Run the checker in the SDK environment:

```bash
python -B process_api/check_sampler.py \
  process_api/examples/python_sampler/src/demo_sampler.py:UniformSampler \
  --require-stratified --statistics --max-samples 1024

python -B process_api/check_sampler.py \
  process_api/examples/python_sampler/src/demo_sampler.py:UniformSampler \
  --init-args '{"training_samples":128,"feedback_window":32}' \
  --stage-init-args '{"training_samples":0}'
```

All recovery state is inline JSON; this example writes no checkpoint files and
needs no commit-notification hook. Ragged domains are deliberately left to more
specialized samplers.
