//! Output-space partition and frozen probability table shared by both worker roles.
use crate::{
    Domain,
    core::{AccumulatorConfig, EngineError},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
        domain: Domain,
        #[serde(with = "prefix_centers")]
        centers: BTreeMap<Vec<i64>, Vec<f64>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scales: Option<Vec<f64>>,
    },
}

impl StratificationConfig {
    pub fn centers(&self) -> &BTreeMap<Vec<i64>, Vec<f64>> {
        match self {
            Self::Voronoi { centers, .. } => centers,
        }
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        let Self::Voronoi {
            centers, scales, ..
        } = self;
        let prefixes = self
            .domain()
            .stratum_prefixes()
            .map_err(EngineError::invalid_input)?;
        self.domain()
            .with_integrand(&Domain::continuous(0))
            .map_err(EngineError::invalid_input)?;
        if !centers.keys().eq(prefixes.iter()) {
            return Err(EngineError::invalid_input(
                "Voronoi centers must exactly cover all complete stratum prefixes in the domain",
            ));
        }
        let width = centers.values().next().map_or(0, Vec::len);
        if width == 0
            || centers
                .values()
                .any(|c| c.len() != width || c.iter().any(|v| !v.is_finite()))
        {
            return Err(EngineError::invalid_input(
                "Voronoi centers must be nonempty finite vectors of equal width",
            ));
        }
        if centers
            .values()
            .enumerate()
            .any(|(i, c)| centers.values().take(i).any(|old| old == c))
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
        if width != self.centers().values().next().map_or(0, Vec::len) {
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
    pub fn domain(&self) -> &Domain {
        match self {
            Self::Voronoi { domain, .. } => domain,
        }
    }
    pub fn prefixes(&self) -> impl ExactSizeIterator<Item = &Vec<i64>> {
        self.centers().keys()
    }
    pub fn prefix(&self, label: usize) -> &[i64] {
        self.prefixes().nth(label).expect("validated stratum label")
    }
    pub fn label(&self, prefix: &[i64]) -> Result<usize, EngineError> {
        self.prefixes()
            .position(|p| p == prefix)
            .ok_or_else(|| EngineError::eval("unknown stratum prefix"))
    }
    pub fn sampling_domain(&self, integrand: &Domain) -> Result<Domain, EngineError> {
        self.validate()?;
        self.domain()
            .with_integrand(integrand)
            .map_err(EngineError::invalid_input)
    }
    pub fn classify(&self, values: &[f64]) -> Result<usize, EngineError> {
        let Self::Voronoi {
            centers, scales, ..
        } = self;
        if values.len() != centers.values().next().map_or(0, Vec::len)
            || values.iter().any(|v| !v.is_finite())
        {
            return Err(EngineError::eval("invalid Voronoi classification values"));
        }
        let mut best = (0, f64::INFINITY);
        for (k, center) in centers.values().enumerate() {
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
        if self.components.len() != self.partition.centers().values().next().map_or(0, Vec::len) {
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

/// Canonical decimal tuple keys work in TOML, JSON and persisted binary payloads.
mod prefix_centers {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        centers: &BTreeMap<Vec<i64>, Vec<f64>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(centers.len()))?;
        for (prefix, center) in centers {
            map.serialize_entry(
                &prefix
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                center,
            )?;
        }
        map.end()
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Vec<i64>, Vec<f64>>, D::Error> {
        let raw = BTreeMap::<String, Vec<f64>>::deserialize(deserializer)?;
        raw.into_iter()
            .map(|(key, value)| {
                let prefix = key
                    .split(',')
                    .map(str::parse::<i64>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(serde::de::Error::custom)?;
                if prefix.iter().any(|v| *v < 0)
                    || prefix
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                        != key
                {
                    return Err(serde::de::Error::custom(
                        "stratum keys must be canonical comma-separated nonnegative integers",
                    ));
                }
                Ok((prefix, value))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaled_voronoi_preserves_units_and_uses_stable_ties() {
        let raw = StratificationConfig::Voronoi {
            domain: crate::Domain::strata(vec![2]),
            centers: vec![vec![0., 0.], vec![2., 10.]]
                .into_iter()
                .enumerate()
                .map(|(k, c)| (vec![k as i64], c))
                .collect(),
            scales: None,
        };
        assert_eq!(raw.classify(&[0., 8.]).unwrap(), 1);
        let scaled = StratificationConfig::Voronoi {
            domain: raw.domain().clone(),
            centers: raw.centers().clone(),
            scales: Some(vec![1., 10.]),
        };
        assert_eq!(scaled.classify(&[0., 8.]).unwrap(), 0);
        assert_eq!(scaled.classify(&[1., 5.]).unwrap(), 0);
        assert!(scaled.classify(&[f64::NAN, 0.]).is_err());
        for bad in [vec![], vec![0., 1.], vec![1., f64::INFINITY]] {
            assert!(
                StratificationConfig::Voronoi {
                    domain: raw.domain().clone(),
                    centers: raw.centers().clone(),
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
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![-1.], vec![1.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
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
