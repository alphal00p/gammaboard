//! Diagnostic decomposition of the current process observable wire path.
//! This deliberately mirrors its copies; it is not an optimized replacement.
#[path = "../../process_api/examples/rust_gammaloop/batch.rs"]
mod worker_batch;

use anyhow::{Result, ensure};
use clap::Parser;
use gammaboard::evaluation::{GammaLoopParams, ObservableTrainingProjection};
use gammaboard::{
    AccumulatorConfig, Batch, Domain, EngineError, EvalBatchOptions, Evaluator, EvaluatorBatch,
    EvaluatorConfig, Point, SampleOutput,
};
use gammalooprs::observables::ObservableSnapshotBundle;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, hint::black_box, path::PathBuf, time::Instant};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    args: PathBuf,
    #[arg(long)]
    worker: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "128,512,1024")]
    batch_sizes: Vec<usize>,
    #[arg(long, default_value_t = 8192)]
    samples: usize,
}

fn timed<T>(times: &mut BTreeMap<String, f64>, label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    *times.entry(label.into()).or_default() += start.elapsed().as_secs_f64();
    result
}

fn halton(mut index: usize, base: usize) -> f64 {
    let (mut value, mut scale) = (0., 1.);
    while index > 0 {
        scale /= base as f64;
        value += scale * (index % base) as f64;
        index /= base;
    }
    value
}

struct Replay {
    domain: Domain,
    projection: ObservableTrainingProjection,
    samples: Vec<SampleOutput>,
}
impl Evaluator for Replay {
    fn descriptor(&self) -> gammaboard::EvaluatorDescriptor {
        let mut d = gammaboard::EvaluatorDescriptor::complex(self.domain.clone(), self.projection);
        d.observables = true;
        d.details = false;
        d
    }
    fn training_projection(&self) -> ObservableTrainingProjection {
        self.projection
    }
    fn evaluate(
        &mut self,
        _: &Batch,
        _: &gammaboard::EvaluationRequest,
        emit: &mut dyn FnMut(SampleOutput) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        for sample in self.samples.drain(..) {
            emit(sample)?;
        }
        Ok(())
    }
}

fn decompose(
    native: &mut dyn Evaluator,
    batch: &Batch,
    times: &mut BTreeMap<String, f64>,
) -> Result<Value> {
    let components = vec!["real".to_string(), "imag".to_string()];
    let domain = native.domain();
    let wire_input = timed(times, "input_pack_and_decode", || -> Result<Batch> {
        let binary: Vec<_> = batch
            .points()
            .iter()
            .flat_map(|p| p.continuous.iter())
            .flat_map(|x| x.to_le_bytes())
            .collect();
        let params = serde_json::from_value::<worker_batch::EvalBatch>(
            json!({"nr_samples":batch.len(),"components":components}),
        )?;
        ensure!(
            params.components == components && params.observable_encoding.is_none(),
            "component or encoding mismatch"
        );
        params.decode(&binary, &domain)
    })?;
    let (values, bundles) = timed(
        times,
        "worker_native_evaluation_and_capture",
        || -> Result<_> {
            let mut values = vec![];
            let mut bundles = vec![];
            native.eval_samples(&wire_input, &components, &mut |sample| {
                values.extend(sample.values);
                bundles.push(sample.observables.expect("native observable bundle"));
                Ok(())
            })?;
            Ok((values, bundles))
        },
    )?;
    ensure!(
        !bundles[0].histograms.is_empty(),
        "fixture must have observables"
    );
    let histograms = bundles[0].histograms.len();
    let bins: usize = bundles[0]
        .histograms
        .values()
        .map(|h| h.bins.len() + 2)
        .sum();
    let result = timed(
        times,
        "worker_build_json_tree",
        || json!({"observables":bundles}),
    );
    // Match the worker's ordering: encode values before dropping its JSON trees.
    // Moving this allocation after the frees can charge allocator consolidation
    // to the tiny binary buffer, rather than to the operations causing it.
    let binary = timed(times, "values_binary_encode", || {
        values
            .into_iter()
            .flat_map(f64::to_le_bytes)
            .collect::<Vec<_>>()
    });
    timed(times, "worker_drop_typed_bundles", || drop(bundles));
    // send_result's json! macro serializes the existing Value again (a deep copy).
    let envelope = timed(
        times,
        "worker_envelope_tree_copy",
        || json!({"jsonrpc":"2.0","id":1,"result":result}),
    );
    let body = timed(times, "worker_encode_json_bytes", || {
        serde_json::to_vec(&envelope)
    })?;
    let payload_bytes = body.len();
    timed(times, "worker_drop_json_trees", || {
        drop(envelope);
        drop(result);
    });
    let envelope: Value = timed(times, "host_parse_json_tree", || {
        serde_json::from_slice(&body)
    })?;
    let result = timed(times, "host_clone_result_tree", || {
        envelope.get("result").cloned().unwrap()
    });
    timed(times, "host_drop_response_tree", || drop(envelope));
    let raw = timed(times, "host_clone_observables_tree", || {
        result.get("observables").cloned().unwrap()
    });
    let bundles: Vec<ObservableSnapshotBundle> = timed(times, "host_decode_typed_bundles", || {
        serde_json::from_value(raw)
    })?;
    let values: Vec<_> = timed(times, "values_binary_decode", || {
        binary
            .as_chunks::<8>()
            .0
            .iter()
            .map(|x| f64::from_le_bytes(*x))
            .collect()
    });
    let mut replay = timed(times, "host_prepare_sample_outputs", || Replay {
        domain,
        projection: native.training_projection(),
        samples: bundles
            .into_iter()
            .zip(values.as_chunks::<2>().0)
            .map(|(bundle, values)| {
                let mut sample = SampleOutput::values(values.to_vec());
                sample.observables = Some(bundle);
                sample
            })
            .collect(),
    });
    let reduced = timed(times, "host_observable_reduction", || {
        replay.eval_batch(
            batch,
            &AccumulatorConfig::Gammaloop,
            EvalBatchOptions::default(),
        )
    })?;
    timed(times, "host_drop_result_tree_and_buffers", || {
        drop(result);
        drop(body);
        drop(binary);
    });
    let reduced = serde_json::to_value(reduced.accumulator)?;
    Ok(
        json!({"payload_bytes":payload_bytes,"histograms_per_sample":histograms,"bins_including_flow_per_sample":bins,"accumulator":reduced}),
    )
}

fn typed_alternative(native: &mut dyn Evaluator, batch: &Batch, calls: usize) -> Result<Value> {
    #[derive(Serialize, Deserialize)]
    struct Payload<T> {
        observables: T,
    }
    #[derive(Serialize, Deserialize)]
    struct Envelope<T> {
        jsonrpc: String,
        id: u64,
        result: T,
    }
    let mut bundles = Vec::new();
    native.eval_samples(batch, &["real".into(), "imag".into()], &mut |sample| {
        bundles.push(sample.observables.unwrap());
        Ok(())
    })?;
    let value = Envelope {
        jsonrpc: "2.0".into(),
        id: 1,
        result: Payload {
            observables: &bundles,
        },
    };
    let bytes = serde_json::to_vec(&value)?;
    let restored: Envelope<Payload<Vec<ObservableSnapshotBundle>>> =
        serde_json::from_slice(&bytes)?;
    ensure!(
        serde_json::to_value(&restored.result.observables)? == serde_json::to_value(&bundles)?,
        "typed roundtrip mismatch"
    );
    drop(restored);
    let mut times = BTreeMap::new();
    for _ in 0..calls {
        timed(&mut times, "encode_and_free_bytes", || -> Result<()> {
            drop(black_box(serde_json::to_vec(&value)?));
            Ok(())
        })?;
        timed(&mut times, "decode_and_free_bundles", || -> Result<()> {
            let decoded: Envelope<Payload<Vec<ObservableSnapshotBundle>>> =
                serde_json::from_slice(&bytes)?;
            drop(black_box(decoded));
            Ok(())
        })?;
    }
    Ok(json!({"seconds":times,"payload_bytes":bytes.len(),"wire_schema_unchanged":true}))
}

fn production_calls(
    config: &EvaluatorConfig,
    batch: &Batch,
    accumulator: &AccumulatorConfig,
    calls: usize,
) -> Result<Value> {
    let start = Instant::now();
    let mut evaluator = config.build()?;
    let initialization_seconds = start.elapsed().as_secs_f64();
    // Warm-up is excluded. Capture a numerical result for native/process parity.
    let reference = serde_json::to_value(
        evaluator
            .eval_batch(batch, accumulator, EvalBatchOptions::default())?
            .accumulator,
    )?;
    let mut durations = Vec::new();
    for _ in 0..calls {
        let start = Instant::now();
        drop(black_box(evaluator.eval_batch(
            batch,
            accumulator,
            EvalBatchOptions::default(),
        )?));
        durations.push(start.elapsed().as_secs_f64());
    }
    Ok(
        json!({"initialization_seconds":initialization_seconds,"seconds":durations,"reference":reference}),
    )
}

fn run(args: Args) -> Result<()> {
    ensure!(
        args.samples > 0 && args.batch_sizes.iter().all(|n| *n > 0 && *n <= 1024),
        "invalid batch sizes or sample budget"
    );
    let params: GammaLoopParams = serde_json::from_slice(&std::fs::read(&args.args)?)?;
    let native_config = EvaluatorConfig::Gammaloop {
        params: params.clone(),
    };
    let mut native = native_config.build()?;
    let domain = native.domain();
    ensure!(
        domain == Domain::continuous(3),
        "this diagnostic uses the 3D gg→HHH state"
    );
    let mut rows = Vec::new();
    for size in args.batch_sizes {
        let calls = args.samples.div_ceil(size);
        let batch = Batch::from_iter(
            (1..=size)
                .map(|i| Point::new([2, 3, 5].map(|base| halton(i, base)).to_vec(), vec![], 1.)),
        );
        let process = |output| -> Result<EvaluatorConfig> {
            Ok(serde_json::from_value(json!({
                "kind":"process_evaluator", "command":[args.worker], "cwd":args.worker.parent(),
                "domain":domain,"args":params,"output":output,
            }))?)
        };
        let vector = AccumulatorConfig::vector(
            vec!["real".into(), "imag".into()],
            gammaboard::TrainingProjection::Norm,
        );
        let mut measurements = BTreeMap::new();
        for (name, config, accumulator) in [
            (
                "native_observables",
                native_config.clone(),
                AccumulatorConfig::Gammaloop,
            ),
            (
                "process_observables",
                process(
                    json!({"kind":"gammaloop", "training_projection":params.training_projection}),
                )?,
                AccumulatorConfig::Gammaloop,
            ),
            ("native_values", native_config.clone(), vector.clone()),
            (
                "process_values",
                process(json!({"kind":"values","components":["real","imag"]}))?,
                vector,
            ),
        ] {
            eprintln!("batch={size}, calls={calls}, measuring {name}");
            measurements.insert(
                name,
                production_calls(&config, &batch, &accumulator, calls)?,
            );
        }
        let a = &measurements["native_observables"]["reference"];
        let b = &measurements["process_observables"]["reference"];
        ensure!(
            a["bundle"] == b["bundle"],
            "native/process histogram mismatch"
        );
        let native_components = measurements["native_values"]["reference"]["components"]
            .as_array()
            .unwrap();
        let process_components = measurements["process_values"]["reference"]["components"]
            .as_array()
            .unwrap();
        ensure!(
            native_components.len() == 2 && process_components.len() == 2,
            "missing vector components"
        );
        for (a, b) in native_components.iter().zip(process_components) {
            for field in [
                "count",
                "sum_weighted_value",
                "sum_abs",
                "sum_sq",
                "sum_cu",
                "sum_four",
            ] {
                ensure!(
                    a["state"][field] == b["state"][field],
                    "native/process vector moment mismatch: {field}"
                );
            }
        }
        let mut times = BTreeMap::new();
        decompose(native.as_mut(), &batch, &mut BTreeMap::new())?;
        let mut frame = Value::Null;
        for _ in 0..calls {
            frame = decompose(native.as_mut(), &batch, &mut times)?;
        }
        ensure!(
            frame["accumulator"]["bundle"] == a["bundle"],
            "replayed reduction differs"
        );
        frame.as_object_mut().unwrap().remove("accumulator");
        let production_sum: f64 = times.values().sum();
        let alternative = typed_alternative(native.as_mut(), &batch, calls)?;
        rows.push(json!({"batch_size":size,"calls":calls,"samples":calls*size,"production":measurements,
            "decomposition_seconds":times,"decomposition_production_sum_seconds":production_sum,"frame":frame,
            "typed_json_alternative":alternative}));
        std::fs::write(
            &args.output,
            serde_json::to_vec_pretty(&json!({"rows":rows}))?,
        )?;
    }
    Ok(())
}

fn main() -> Result<()> {
    run(Args::parse())
}
