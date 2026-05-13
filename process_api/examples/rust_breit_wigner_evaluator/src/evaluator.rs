use serde_json::Value;

#[derive(Clone, Debug)]
pub struct EvaluatorConfig {
    pub masses: Vec<f64>,
    pub widths: Vec<f64>,
    pub channel_weights: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct BreitWignerEvaluator {
    config: EvaluatorConfig,
}

impl BreitWignerEvaluator {
    pub fn from_args(
        args: &Value,
        discrete_cardinalities: &[usize],
        continuous_dims: usize,
    ) -> Result<Self, String> {
        if continuous_dims != 2 {
            return Err(format!(
                "rust breit-wigner evaluator expects continuous_dims=2, got {continuous_dims}"
            ));
        }
        if discrete_cardinalities.len() != 1 {
            return Err(format!(
                "rust breit-wigner evaluator expects one discrete axis, got {}",
                discrete_cardinalities.len()
            ));
        }
        let channels = discrete_cardinalities[0];
        if channels == 0 {
            return Err("discrete channel cardinality must be > 0".to_string());
        }

        let masses = read_f64_array(args, "masses", channels, 0.5)?;
        let widths = read_f64_array(args, "widths", channels, 0.05)?;
        let channel_weights = read_f64_array(args, "channel_weights", channels, 1.0)?;
        if widths
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err("widths must be finite and > 0".to_string());
        }

        Ok(Self {
            config: EvaluatorConfig {
                masses,
                widths,
                channel_weights,
            },
        })
    }

    pub fn eval_batch(
        &self,
        xs_discrete: &[i64],
        xs_continuous: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, String> {
        if xs_discrete.len() != nr_samples {
            return Err(format!(
                "expected {nr_samples} discrete entries, got {}",
                xs_discrete.len()
            ));
        }
        if xs_continuous.len() != nr_samples * 2 {
            return Err(format!(
                "expected {} continuous entries, got {}",
                nr_samples * 2,
                xs_continuous.len()
            ));
        }

        let mut values = Vec::with_capacity(nr_samples);
        for sample_index in 0..nr_samples {
            let channel = xs_discrete[sample_index];
            if channel < 0 || channel as usize >= self.config.masses.len() {
                return Err(format!(
                    "channel {channel} out of bounds for {} channels",
                    self.config.masses.len()
                ));
            }
            let channel = channel as usize;
            let x = xs_continuous[2 * sample_index];
            let y = xs_continuous[2 * sample_index + 1];
            if !x.is_finite() || !y.is_finite() {
                return Err("continuous inputs must be finite".to_string());
            }
            values.push(breit_wigner_value(
                channel,
                x,
                y,
                &self.config.masses,
                &self.config.widths,
                &self.config.channel_weights,
            ));
        }
        Ok(values)
    }
}

pub fn breit_wigner_value(
    channel: usize,
    x: f64,
    y: f64,
    masses: &[f64],
    widths: &[f64],
    channel_weights: &[f64],
) -> f64 {
    let dx = x - masses[channel];
    channel_weights[channel] * (-y).exp() / (dx * dx + widths[channel] * widths[channel])
}

fn read_f64_array(
    args: &Value,
    key: &str,
    expected_len: usize,
    default: f64,
) -> Result<Vec<f64>, String> {
    let Some(raw) = args.get(key) else {
        return Ok(vec![default; expected_len]);
    };
    let Some(items) = raw.as_array() else {
        return Err(format!("args.{key} must be an array"));
    };
    if items.len() != expected_len {
        return Err(format!(
            "args.{key} must have length {expected_len}, got {}",
            items.len()
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("args.{key}[{index}] must be a finite number"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{breit_wigner_value, BreitWignerEvaluator};

    #[test]
    fn evaluates_channel_specific_resonance() {
        let args = serde_json::json!({
            "masses": [0.25, 0.50],
            "widths": [0.05, 0.10],
            "channel_weights": [1.0, 2.0],
        });
        let evaluator = BreitWignerEvaluator::from_args(&args, &[2], 2).unwrap();
        let values = evaluator
            .eval_batch(&[0, 1], &[0.25, 0.0, 0.50, 0.0], 2)
            .unwrap();

        assert!((values[0] - 400.0).abs() < 1e-12);
        assert!((values[1] - 200.0).abs() < 1e-12);
    }

    #[test]
    fn pure_function_is_easy_to_copy() {
        let value = breit_wigner_value(0, 0.25, 0.0, &[0.25], &[0.05], &[1.0]);
        assert!((value - 400.0).abs() < 1e-12);
    }
}
