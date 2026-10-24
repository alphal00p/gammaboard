//! Reproducible A/B experiment using the production Symbolica/Havana pipeline.
use anyhow::{Context, Result, bail};
use gammaboard::core::{StratificationConfig, StratificationPlan, TrainingProjection};
use gammaboard::evaluation::stratification::integrand_batch;
use gammaboard::evaluation::{AccumulatorState, EvalBatchOptions};
use gammaboard::sampling::{DiscreteSubspace, Generation, StageHandoff};
use gammaboard::{AccumulatorConfig, Batch, Evaluator, EvaluatorConfig, SamplingConfig};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};

fn value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.windows(2).find(|p| p[0] == key).map(|p| p[1].as_str())
}
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let trials: usize = value(&args, "--trials").unwrap_or("32").parse()?;
    let training: usize = value(&args, "--training").unwrap_or("131072").parse()?;
    let inference: usize = value(&args, "--inference").unwrap_or("131072").parse()?;
    let batch_size: usize = value(&args, "--batch-size").unwrap_or("2048").parse()?;
    let output = PathBuf::from(value(&args, "--output").unwrap_or("stratification-ab.json"));
    anyhow::ensure!(
        trials >= 2 && training > 0 && inference > 0 && batch_size > 0,
        "positive budgets/batch size and at least two trials required"
    );
    let components = vec!["x".into(), "y".into(), "z".into()];
    let centers = [
        vec![1., 1., 1.],
        vec![-1., 1., -1.],
        vec![-1., -1., 1.],
        vec![1., -1., -1.],
    ];
    // Smooth traversal close to the tetrahedron's edges; tanh slows near vertices.
    // Half/quarter-period symmetries make the exact vector integral [0,0,0].
    let x = "(exp(16*cos(2*pi*t))-1)/(exp(16*cos(2*pi*t))+1)";
    let y = "(exp(16*sin(2*pi*t))-1)/(exp(16*sin(2*pi*t))+1)";
    let exprs = vec![x.to_string(), y.to_string(), format!("({x})*({y})")];
    let evaluator_config: EvaluatorConfig = serde_json::from_value(
        json!({"kind":"symbolica","args":["t"],"source":{"kind":"expressions","components":components,"exprs":exprs}}),
    )?;
    let mut evaluator = evaluator_config.build()?;
    let domain = evaluator.domain();
    let accumulator = AccumulatorConfig::vector(components.clone(), TrainingProjection::Norm);
    let partition = StratificationConfig::Voronoi {
        domain: gammaboard::Domain::strata(vec![2, 2]),
        centers: centers
            .iter()
            .enumerate()
            .map(|(k, c)| (vec![(k / 2) as i64, (k % 2) as i64], c.clone()))
            .collect(),
        scales: Some(vec![1., 1., 1.]),
    };
    let mut records = vec![];
    for trial in 0..trials {
        // Alternate order to reduce warm-cache and load effects.
        for stratified in if trial % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = Instant::now();
            let mut config: SamplingConfig = serde_json::from_value(
                json!({"kind":"havana_training","seed":10000+trial,"bins":64,"samples_for_update":4096,"initial_training_rate":0.5,"final_training_rate":0.1}),
            )?;
            if stratified {
                config.stratification = Some(partition.clone());
            }
            let mut sampler = config.create(domain.clone(), Some(training), json!({}))?;
            let mut materializer = config.build_materializer(&domain, None)?;
            let training_plan = stratified.then(|| StratificationPlan {
                proposal_id: format!("train-{trial}"),
                partition: partition.clone(),
                probabilities: None,
                components: components.clone(),
            });
            let mut generated = 0;
            while generated < training {
                let Generation::Batch {
                    batch,
                    training_remaining,
                } = sampler.generate(batch_size.min(training - generated))?
                else {
                    bail!("training stopped prematurely")
                };
                let size = batch.nr_samples;
                let concrete = materializer.materialize_batch(&batch)?;
                let result = evaluate(
                    &mut *evaluator,
                    concrete,
                    &accumulator,
                    training_remaining.is_some(),
                    training_plan.as_ref(),
                )?;
                if let Some(feedback) = result.feedback {
                    sampler.feedback(&feedback)?;
                }
                generated += size;
            }
            let saved = config.snapshot(&mut *sampler)?;
            let config: SamplingConfig =
                serde_json::from_value(json!({"kind":"havana_inference","seed":100000+trial}))?;
            let config = config.inherit_stratification(&saved)?;
            let handoff = StageHandoff {
                sampler_snapshot: Some(&saved),
                observable_state: None,
            };
            let mut sampler = config.from_stage(&domain, handoff)?;
            let mut materializer = config.build_materializer(&domain, Some(handoff))?;
            let plan = if stratified {
                let queries = partition
                    .prefixes()
                    .map(|prefix| DiscreteSubspace {
                        fixed_dims: prefix.iter().copied().enumerate().collect(),
                    })
                    .collect::<Vec<_>>();
                let probabilities = sampler
                    .discrete_mass_batch(&queries)?
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .context("missing discrete probabilities")?;
                let plan = StratificationPlan {
                    proposal_id: format!("inference-{trial}"),
                    partition: partition.clone(),
                    probabilities: Some(probabilities),
                    components: components.clone(),
                };
                plan.validate()?;
                Some(plan)
            } else {
                None
            };
            let train_seconds = started.elapsed().as_secs_f64();
            let inference_started = Instant::now();
            let mut state = if let Some(plan) = &plan {
                AccumulatorState::Stratified(Box::new(
                    gammaboard::evaluation::accumulator::StratifiedAccumulatorState::new(
                        plan.clone(),
                        accumulator.clone(),
                    )?,
                ))
            } else {
                AccumulatorState::from_config(&accumulator)
            };
            generated = 0;
            while generated < inference {
                let Generation::Batch {
                    batch,
                    training_remaining,
                } = sampler.generate(batch_size.min(inference - generated))?
                else {
                    bail!("inference stopped prematurely")
                };
                anyhow::ensure!(training_remaining.is_none(), "inference requested training");
                let size = batch.nr_samples;
                let concrete = materializer.materialize_batch(&batch)?;
                let result = evaluate(
                    &mut *evaluator,
                    concrete,
                    &accumulator,
                    false,
                    plan.as_ref(),
                )?;
                state.merge(result.accumulator)?;
                generated += size;
            }
            let mut estimates = vec![];
            let mut errors = vec![];
            for component in &components {
                let measurement = gammaboard::evaluation::extract_accumulator_metric(
                    &state,
                    &gammaboard::core::AccumulatorMetricSelector {
                        name: gammaboard::core::AccumulatorMetricName::Mean,
                        component: Some(component.clone()),
                    },
                )?
                .context("no final estimate")?;
                estimates.push(measurement.value);
                errors.push(measurement.uncertainty.context("no final uncertainty")?);
            }
            let strata = match &state {
                AccumulatorState::Stratified(s) => json!(s.strata),
                _ => Value::Null,
            };
            let record = json!({"trial":trial,"arm":if stratified{"stratified"}else{"ordinary"},"estimate":estimates,"stderr":errors,"training_samples":training,"inference_samples":inference,"training_seconds":train_seconds,"inference_seconds":inference_started.elapsed().as_secs_f64(),"probabilities":plan.as_ref().and_then(|p|p.probabilities.as_ref()),"strata":strata});
            eprintln!(
                "trial={trial} arm={} estimate={estimates:?} stderr={errors:?}",
                if stratified { "stratified" } else { "ordinary" }
            );
            records.push(record);
        }
    }
    let result = json!({"schema":"gammaboard-stratification-ab-v2","integrand":{"exprs":exprs,"stratification":partition,"domain":[0.,1.],"exact_integral":[0.,0.,0.]},"configuration":{"trials":trials,"training_samples":training,"inference_samples":inference,"batch_size":batch_size,"havana_bins":64,"training_window":4096},"build":gammaboard::provenance::build_info(),"records":records});
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    eprintln!("saved {}", output.display());
    Ok(())
}
fn evaluate(
    evaluator: &mut dyn Evaluator,
    batch: Batch,
    config: &AccumulatorConfig,
    training: bool,
    plan: Option<&StratificationPlan>,
) -> Result<gammaboard::BatchResult> {
    let options = EvalBatchOptions {
        require_training_values: training,
        stratification: None,
    };
    Ok(if let Some(plan) = plan {
        let (integrand, labels) = integrand_batch(batch, plan)?;
        evaluator.eval_batch(
            &integrand,
            config,
            EvalBatchOptions {
                stratification: Some((plan, &labels)),
                ..options
            },
        )?
    } else {
        evaluator.eval_batch(&batch, config, options)?
    })
}
