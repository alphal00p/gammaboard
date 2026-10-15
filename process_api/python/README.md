# GammaBoard Process API

Python helpers for implementing GammaBoard process evaluators, samplers, batch
transforms, and materializers over the `gammaboard-jsonrpc-v6` protocol.

- `Evaluator.eval` returns unweighted values; `GammaLoopEvaluator.eval_gammaloop`
  returns weighted observable state and requested feedback.
- `Sampler.generate` returns `SampleBatch`, `WAITING`, or `FINISHED`.
- `Materializer.materialize_batch` and `BatchTransform.transform_batch` return
  the same `Batch` type.
- Evaluator metadata and worker diagnostics use `metadata()` and `diagnostics()`.
  Samplers may also override `global_pdf_norm()` and `persisted_output()`.

See the [process API documentation](../README.md) in the GammaBoard repository
for protocol details and examples.
