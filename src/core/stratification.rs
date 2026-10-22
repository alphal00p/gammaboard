//! Output-space partition and frozen probability table shared by both worker roles.
use crate::{
    Domain,
    core::{AccumulatorConfig, EngineError},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Estimator {
    #[default]
    Pooled,
    Stratified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StratificationConfig {
    Voronoi {
        centers: Vec<Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scales: Option<Vec<f64>>,
    },
}

impl StratificationConfig {
    pub fn centers(&self) -> &[Vec<f64>] {
        match self {
            Self::Voronoi { centers, .. } => centers,
        }
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        let Self::Voronoi { centers, scales } = self;
        let width = centers.first().map_or(0, Vec::len);
        if width == 0
            || centers
                .iter()
                .any(|c| c.len() != width || c.iter().any(|v| !v.is_finite()))
        {
            return Err(EngineError::invalid_input(
                "Voronoi centers must be nonempty finite vectors of equal width",
            ));
        }
        if centers
            .iter()
            .enumerate()
            .any(|(i, c)| centers[..i].contains(c))
        {
            return Err(EngineError::invalid_input(
                "Voronoi centers must be distinct",
            ));
        }
        if scales
            .as_ref()
            .is_some_and(|s| s.len() != width || s.iter().any(|v| !v.is_finite() || *v <= 0.0))
        {
            return Err(EngineError::invalid_input(
                "Voronoi scales must match the output width and be finite and positive",
            ));
        }
        Ok(())
    }
    pub fn validate_accumulator(&self, config: &AccumulatorConfig) -> Result<(), EngineError> {
        self.validate()?;
        let width = Self::components(config)?.len();
        if width != self.centers()[0].len() {
            return Err(EngineError::invalid_input(
                "Voronoi center width must match the evaluator output components",
            ));
        }
        Ok(())
    }
    pub fn components(config: &AccumulatorConfig) -> Result<Vec<String>, EngineError> {
        match config {
            AccumulatorConfig::Vector { components, .. } => Ok(components.clone()),
            AccumulatorConfig::Gammaloop => Ok(vec!["real".into(), "imag".into()]),
            _ => Err(EngineError::invalid_input(
                "stratification requires a statistical scalar/vector or GammaLoop accumulator",
            )),
        }
    }
    pub fn sampling_domain(&self, physical: &Domain) -> Domain {
        // An outer branch gives Havana a separate conditional grid per stratum.
        Domain::discrete(
            Some("stratum".into()),
            self.centers().iter().enumerate().map(|(index, _)| {
                crate::utils::domain::DomainBranch {
                    index,
                    domain: Box::new(physical.clone()),
                }
            }),
        )
    }
    pub fn classify(&self, values: &[f64]) -> Result<usize, EngineError> {
        let Self::Voronoi { centers, scales } = self;
        if values.len() != centers[0].len() || values.iter().any(|v| !v.is_finite()) {
            return Err(EngineError::eval("invalid Voronoi classification values"));
        }
        let mut best = (0, f64::INFINITY);
        for (k, center) in centers.iter().enumerate() {
            let distance =
                values
                    .iter()
                    .zip(center)
                    .enumerate()
                    .try_fold(0.0_f64, |norm, (j, (v, c))| {
                        let delta = (v - c) / scales.as_ref().map_or(1.0, |s| s[j]);
                        if !delta.is_finite() {
                            return Err(EngineError::eval("Voronoi distance overflow"));
                        }
                        Ok(norm.hypot(delta))
                    })?;
            if !distance.is_finite() {
                return Err(EngineError::eval("Voronoi distance overflow"));
            }
            if distance < best.1 {
                best = (k, distance);
            }
        }
        Ok(best.0)
    }
}

/// None probabilities means masked, pooled training. Some means frozen inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StratificationPlan {
    pub proposal_id: String,
    pub partition: StratificationConfig,
    pub probabilities: Option<Vec<f64>>,
    pub components: Vec<String>,
}
impl StratificationPlan {
    pub fn validate(&self) -> Result<(), EngineError> {
        if self.proposal_id.is_empty() {
            return Err(EngineError::invalid_input(
                "stratification plan requires a proposal identity",
            ));
        }
        self.partition.validate()?;
        if self.components.len() != self.partition.centers()[0].len() {
            return Err(EngineError::invalid_input(
                "stratification plan component layout mismatch",
            ));
        }
        if let Some(p) = &self.probabilities
            && (p.len() != self.partition.centers().len()
                || p.iter().any(|p| !p.is_finite() || *p <= 0.0 || *p > 1.0)
                || (p.iter().sum::<f64>() - 1.0).abs() > 1e-12)
        {
            return Err(EngineError::invalid_input(
                "stratum probabilities must be finite, positive, complete and normalized",
            ));
        }
        Ok(())
    }
    pub fn matches(&self, label: usize, values: &[f64]) -> Result<bool, EngineError> {
        if label >= self.partition.centers().len() {
            return Err(EngineError::eval("invalid stratum label"));
        }
        Ok(self.partition.classify(values)? == label)
    }
    pub fn id(&self) -> String {
        // Canonical immutable definition; equality also guards every accumulator merge.
        use sha2::{Digest, Sha256};
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("validated plan"))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaled_voronoi_preserves_units_and_uses_stable_ties() {
        let raw = StratificationConfig::Voronoi {
            centers: vec![vec![0., 0.], vec![2., 10.]],
            scales: None,
        };
        assert_eq!(raw.classify(&[0., 8.]).unwrap(), 1);
        let scaled = StratificationConfig::Voronoi {
            centers: raw.centers().to_vec(),
            scales: Some(vec![1., 10.]),
        };
        assert_eq!(scaled.classify(&[0., 8.]).unwrap(), 0);
        assert_eq!(scaled.classify(&[1., 5.]).unwrap(), 0);
        assert!(scaled.classify(&[f64::NAN, 0.]).is_err());
        for bad in [vec![], vec![0., 1.], vec![1., f64::INFINITY]] {
            assert!(
                StratificationConfig::Voronoi {
                    centers: raw.centers().to_vec(),
                    scales: Some(bad)
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn frozen_probabilities_are_complete_and_never_silently_renormalized() {
        let plan = StratificationPlan {
            proposal_id: "p".into(),
            partition: StratificationConfig::Voronoi {
                centers: vec![vec![-1.], vec![1.]],
                scales: None,
            },
            probabilities: Some(vec![0.2, 0.8]),
            components: vec!["value".into()],
        };
        plan.validate().unwrap();
        for probabilities in [
            vec![],
            vec![1.],
            vec![0., 1.],
            vec![-0.1, 1.1],
            vec![0.2, 0.7],
            vec![f64::NAN, 0.5],
        ] {
            assert!(
                StratificationPlan {
                    probabilities: Some(probabilities),
                    ..plan.clone()
                }
                .validate()
                .is_err()
            );
        }
        let different = StratificationPlan {
            proposal_id: "new-stage".into(),
            ..plan.clone()
        };
        assert_ne!(plan.id(), different.id());
    }
}
