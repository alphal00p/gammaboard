use serde_json::Value;

#[derive(Clone, Debug)]
pub struct EvaluatorConfig {
    pub scale: f64,
}

#[derive(Clone, Debug)]
pub struct BranchingDomainEvaluator {
    config: EvaluatorConfig,
}

impl BranchingDomainEvaluator {
    pub fn from_args(args: &Value) -> Result<Self, String> {
        let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(1.0);
        if !scale.is_finite() {
            return Err("args.scale must be finite".to_string());
        }
        Ok(Self {
            config: EvaluatorConfig { scale },
        })
    }

    pub fn eval_batch(
        &self,
        xs_discrete: &[i64],
        xs_discrete_offsets: &[usize],
        xs_continuous: &[f64],
        xs_continuous_offsets: &[usize],
        nr_samples: usize,
    ) -> Result<Vec<f64>, String> {
        validate_offsets("xs_discrete_offsets", xs_discrete_offsets, nr_samples, xs_discrete.len())?;
        validate_offsets(
            "xs_continuous_offsets",
            xs_continuous_offsets,
            nr_samples,
            xs_continuous.len(),
        )?;

        let mut values = Vec::with_capacity(nr_samples);
        for sample_index in 0..nr_samples {
            let discrete =
                &xs_discrete[xs_discrete_offsets[sample_index]..xs_discrete_offsets[sample_index + 1]];
            let continuous = &xs_continuous
                [xs_continuous_offsets[sample_index]..xs_continuous_offsets[sample_index + 1]];
            values.push(self.config.scale * branching_domain_value(discrete, continuous)?);
        }
        Ok(values)
    }
}

pub fn branching_domain_value(discrete: &[i64], continuous: &[f64]) -> Result<f64, String> {
    if continuous.iter().any(|value| !value.is_finite()) {
        return Err("continuous inputs must be finite".to_string());
    }
    match discrete {
        [0] => {
            require_continuous_dims(continuous, 3, discrete)?;
            Ok(continuous[0] + 2.0 * continuous[1] + 3.0 * continuous[2])
        }
        [1, 0] => {
            require_continuous_dims(continuous, 1, discrete)?;
            Ok(10.0 + continuous[0] * continuous[0])
        }
        [1, 1, d2] if (0..=4).contains(d2) => {
            require_continuous_dims(continuous, 5, discrete)?;
            let branch_shift = 20.0 + *d2 as f64;
            Ok(branch_shift
                + continuous
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (index as f64 + 1.0) * value)
                    .sum::<f64>())
        }
        other => Err(format!("unsupported discrete path {other:?}")),
    }
}

fn require_continuous_dims(
    continuous: &[f64],
    expected: usize,
    discrete: &[i64],
) -> Result<(), String> {
    if continuous.len() != expected {
        return Err(format!(
            "discrete path {discrete:?} expects {expected} continuous dimensions, got {}",
            continuous.len()
        ));
    }
    Ok(())
}

fn validate_offsets(
    label: &str,
    offsets: &[usize],
    nr_samples: usize,
    values_len: usize,
) -> Result<(), String> {
    if offsets.len() != nr_samples + 1 {
        return Err(format!("{label} must contain nr_samples + 1 entries"));
    }
    if offsets.first().copied() != Some(0) {
        return Err(format!("{label} must start at 0"));
    }
    if offsets.last().copied() != Some(values_len) {
        return Err(format!("{label} must end at value length {values_len}"));
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(format!("{label} must be non-decreasing"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{branching_domain_value, BranchingDomainEvaluator};

    #[test]
    fn evaluates_all_branch_shapes() {
        assert_eq!(branching_domain_value(&[0], &[1.0, 2.0, 3.0]).unwrap(), 14.0);
        assert_eq!(branching_domain_value(&[1, 0], &[4.0]).unwrap(), 26.0);
        assert_eq!(
            branching_domain_value(&[1, 1, 3], &[1.0, 1.0, 1.0, 1.0, 1.0]).unwrap(),
            38.0
        );
    }

    #[test]
    fn evaluator_consumes_ragged_row_major_batches() {
        let evaluator = BranchingDomainEvaluator::from_args(&serde_json::json!({"scale": 2.0}))
            .expect("evaluator");
        let values = evaluator
            .eval_batch(
                &[0, 1, 0, 1, 1, 4],
                &[0, 1, 3, 6],
                &[1.0, 2.0, 3.0, 4.0, 1.0, 1.0, 1.0, 1.0, 1.0],
                &[0, 3, 4, 9],
                3,
            )
            .expect("values");
        assert_eq!(values, [28.0, 52.0, 78.0]);
    }
}
