//! Independent stratum estimates with stable parallel covariance accumulation.
use crate::core::{
    AccumulatorConfig, AccumulatorMetricName, AccumulatorMetricSelector, EngineError,
    MeasurementResult, StratificationPlan, TrainingProjection,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StratumMoments {
    pub attempts: u64,
    pub matches: u64,
    pub mean: Vec<f64>,
    pub m2: Vec<f64>,
    pub extrema: Vec<crate::evaluation::batch::ContributionExtrema>,
}
impl StratumMoments {
    /// One publication rule shared by measurements, controllers and task panels.
    pub fn uncertainty_ready(&self) -> bool {
        self.attempts >= 32 && self.matches >= 2
    }

    pub fn standard_error(&self, component: usize) -> Option<f64> {
        if !self.uncertainty_ready() || component >= self.mean.len() {
            return None;
        }
        Some(
            (self.m2[component * self.mean.len() + component].max(0.0)
                / (self.attempts as f64 * (self.attempts - 1) as f64))
                .sqrt(),
        )
    }

    fn new(width: usize) -> Self {
        Self {
            attempts: 0,
            matches: 0,
            mean: vec![0.; width],
            m2: vec![0.; width * width],
            extrema: vec![Default::default(); width],
        }
    }
    fn push(&mut self, values: &[f64], matched: bool) -> Result<(), EngineError> {
        if values.len() != self.mean.len() || values.iter().any(|v| !v.is_finite()) {
            return Err(EngineError::eval("invalid stratified contribution"));
        }
        let n = self
            .attempts
            .checked_add(1)
            .ok_or_else(|| EngineError::eval("stratum count overflow"))?;
        let delta = values
            .iter()
            .zip(&self.mean)
            .map(|(x, m)| x - m)
            .collect::<Vec<_>>();
        let next = self
            .mean
            .iter()
            .zip(&delta)
            .map(|(m, d)| m + d / n as f64)
            .collect::<Vec<_>>();
        let width = next.len();
        let mut m2 = self.m2.clone();
        for i in 0..width {
            for j in 0..width {
                m2[i * width + j] += delta[i] * (values[j] - next[j]);
            }
        }
        if next.iter().chain(&m2).any(|v| !v.is_finite()) {
            return Err(EngineError::eval("stratified moment overflow"));
        }
        self.attempts = n;
        self.matches += u64::from(matched);
        self.mean = next;
        self.m2 = m2;
        Ok(())
    }
    fn merge(&mut self, other: &Self) -> Result<(), EngineError> {
        if other.attempts == 0 {
            return Ok(());
        }
        if self.attempts == 0 {
            *self = other.clone();
            return Ok(());
        }
        let n = self
            .attempts
            .checked_add(other.attempts)
            .ok_or_else(|| EngineError::eval("stratum count overflow"))?;
        let a = self.attempts as f64;
        let b = other.attempts as f64;
        let delta = other
            .mean
            .iter()
            .zip(&self.mean)
            .map(|(r, l)| r - l)
            .collect::<Vec<_>>();
        let width = self.mean.len();
        for i in 0..width {
            self.mean[i] += delta[i] * b / n as f64;
            for j in 0..width {
                self.m2[i * width + j] +=
                    other.m2[i * width + j] + delta[i] * delta[j] * (a / n as f64) * b;
            }
        }
        self.attempts = n;
        self.matches += other.matches;
        for (left, right) in self.extrema.iter_mut().zip(&other.extrema) {
            left.merge(right);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StratifiedAccumulatorState {
    pub plan: StratificationPlan,
    pub config: AccumulatorConfig,
    pub strata: Vec<StratumMoments>,
    /// Native histograms retain masked full-weight, pooled statistics.
    pub pooled: Option<Box<super::GammaLoopAccumulatorState>>,
}
impl<'de> Deserialize<'de> for StratifiedAccumulatorState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Saved {
            plan: StratificationPlan,
            config: AccumulatorConfig,
            strata: Vec<StratumMoments>,
            pooled: Option<Box<super::GammaLoopAccumulatorState>>,
        }
        let saved = Saved::deserialize(deserializer)?;
        let state = Self {
            plan: saved.plan,
            config: saved.config,
            strata: saved.strata,
            pooled: saved.pooled,
        };
        state.validate().map_err(serde::de::Error::custom)?;
        Ok(state)
    }
}

impl StratifiedAccumulatorState {
    fn validated_width(
        plan: &StratificationPlan,
        config: &AccumulatorConfig,
    ) -> Result<usize, EngineError> {
        plan.validate()?;
        config.validate().map_err(EngineError::invalid_input)?;
        plan.partition.validate_accumulator(config)?;
        if plan.probabilities.is_none() {
            return Err(EngineError::invalid_input(
                "stratified accumulator needs frozen probabilities",
            ));
        }
        let layout_matches = match config {
            AccumulatorConfig::Gammaloop => plan.components == ["real", "imag"],
            _ => config.components() == Some(plan.components.as_slice()),
        };
        if !layout_matches {
            return Err(EngineError::invalid_input(
                "stratification component mismatch",
            ));
        }
        Ok(if matches!(config, AccumulatorConfig::Gammaloop) {
            4
        } else {
            plan.components.len()
                + usize::from(matches!(
                    config,
                    AccumulatorConfig::Vector {
                        training_projection: TrainingProjection::Norm,
                        ..
                    }
                ))
        })
    }

    pub fn new(plan: StratificationPlan, config: AccumulatorConfig) -> Result<Self, EngineError> {
        let width = Self::validated_width(&plan, &config)?;
        let strata = vec![StratumMoments::new(width); plan.partition.centers().len()];
        Ok(Self {
            plan,
            pooled: matches!(config, AccumulatorConfig::Gammaloop).then(Default::default),
            config,
            strata,
        })
    }
    pub fn push(
        &mut self,
        label: usize,
        values: &[f64],
        weight: f64,
        matched: bool,
    ) -> Result<(), EngineError> {
        self.push_with_diagnostics(label, values, weight, matched, None)
    }

    pub(crate) fn push_with_diagnostics(
        &mut self,
        label: usize,
        values: &[f64],
        weight: f64,
        matched: bool,
        diagnostic: Option<(&crate::Point, crate::evaluation::batch::StratumPair<'_>)>,
    ) -> Result<(), EngineError> {
        if values.len() != self.plan.components.len()
            || label >= self.strata.len()
            || !weight.is_finite()
            || weight <= 0.0
        {
            return Err(EngineError::eval(
                "invalid stratum label, weight or output width",
            ));
        }
        let mut contribution = values.to_vec();
        if matches!(self.config, AccumulatorConfig::Gammaloop) {
            let norm = values.iter().fold(0.0_f64, |n, v| n.hypot(*v));
            contribution.extend([norm, norm * norm]);
        }
        if matches!(
            &self.config,
            AccumulatorConfig::Vector {
                training_projection: TrainingProjection::Norm,
                ..
            }
        ) {
            contribution.push(values.iter().fold(0.0_f64, |n, v| n.hypot(*v)));
        }
        let p = self
            .plan
            .probabilities
            .as_ref()
            .expect("validated inference plan")[label];
        for x in &mut contribution {
            *x *= weight * p;
        }
        self.strata[label].push(&contribution, matched)?;
        if let Some((point, assignment)) = diagnostic {
            for (extrema, &value) in self.strata[label].extrema.iter_mut().zip(&contribution) {
                extrema.observe(value, point, assignment);
            }
        }
        Ok(())
    }
    pub fn sample_count(&self) -> i64 {
        self.strata.iter().map(|s| s.attempts as i64).sum()
    }
    pub fn validate(&self) -> Result<(), EngineError> {
        let width = Self::validated_width(&self.plan, &self.config)?;
        if self.strata.len() != self.plan.partition.centers().len() {
            return Err(EngineError::eval("incomplete stratum state"));
        }
        let mut count = 0u64;
        for s in &self.strata {
            if s.mean.len() != width
                || s.m2.len() != width * width
                || s.extrema.len() != width
                || s.matches > s.attempts
                || s.mean.iter().chain(&s.m2).any(|v| !v.is_finite())
            {
                return Err(EngineError::eval("invalid stratum moments"));
            }
            count = count
                .checked_add(s.attempts)
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(|| EngineError::eval("stratum count overflow"))?;
        }
        match &self.pooled {
            Some(pooled) if matches!(self.config, AccumulatorConfig::Gammaloop) => {
                pooled.validate_values()?;
                pooled.validate_stratified_samples(count as i64)?;
            }
            None if !matches!(self.config, AccumulatorConfig::Gammaloop) => {}
            _ => return Err(EngineError::eval("incompatible pooled observable state")),
        }
        Ok(())
    }
    pub fn merge(&mut self, other: Self) -> Result<(), EngineError> {
        self.validate()?;
        other.validate()?;
        if self.plan != other.plan || self.config != other.config {
            return Err(EngineError::eval("incompatible stratification plans"));
        }
        let mut merged = self.clone();
        for (a, b) in merged.strata.iter_mut().zip(&other.strata) {
            a.merge(b)?;
        }
        if let (Some(left), Some(right)) = (&mut merged.pooled, other.pooled) {
            left.merge_in_place(*right)?;
        }
        merged.validate()?;
        *self = merged;
        Ok(())
    }
    pub fn mean(&self) -> Option<Vec<f64>> {
        if self.strata.iter().any(|s| s.attempts == 0) {
            return None;
        }
        let mut mean = vec![0.; self.strata[0].mean.len()];
        for s in &self.strata {
            for (a, b) in mean.iter_mut().zip(&s.mean) {
                *a += b;
            }
        }
        Some(mean)
    }
    pub fn covariance(&self) -> Option<Vec<f64>> {
        if !self.uncertainty_ready() {
            return None;
        }
        let mut covariance = vec![0.; self.strata[0].m2.len()];
        for s in &self.strata {
            let divisor = (s.attempts as f64) * (s.attempts - 1) as f64;
            for (a, b) in covariance.iter_mut().zip(&s.m2) {
                *a += b / divisor;
            }
        }
        Some(covariance)
    }

    /// Pooled reference from the same masked draws. Scale each conditional
    /// group back by 1/p, then merge central moments without subtracting raw
    /// squared sums. This is a different mean, not another error on `mean()`.
    pub fn basic_estimates(&self) -> Option<Vec<crate::core::BasicEstimate>> {
        let width = self.strata[0].mean.len();
        let mut pooled = StratumMoments::new(width);
        for (stratum, &p) in self.strata.iter().zip(self.plan.probabilities.as_ref()?) {
            let mut group = StratumMoments::new(width);
            group.attempts = stratum.attempts;
            group.mean = stratum.mean.iter().map(|v| v / p).collect();
            group.m2 = stratum.m2.iter().map(|v| (v / p) / p).collect();
            if group.mean.iter().chain(&group.m2).any(|v| !v.is_finite()) {
                return None;
            }
            pooled.merge(&group).ok()?;
            if pooled.mean.iter().chain(&pooled.m2).any(|v| !v.is_finite()) {
                return None;
            }
        }
        let n = pooled.attempts;
        if n == 0 {
            return None;
        }
        Some(
            pooled
                .mean
                .iter()
                .enumerate()
                .map(|(j, &mean)| crate::core::BasicEstimate {
                    mean,
                    error: (n >= 2).then(|| {
                        (pooled.m2[j * width + j].max(0.0) / (n as f64 * (n - 1) as f64)).sqrt()
                    }),
                })
                .collect(),
        )
    }
    pub fn metric(&self, selector: &AccumulatorMetricSelector) -> Option<MeasurementResult> {
        let component = match (selector.component.as_deref(), self.pooled.as_deref()) {
            (None | Some("training_projection"), Some(pooled)) => {
                use crate::evaluation::evaluator::gammaloop::TrainingProjection;
                Some(match pooled.training_projection {
                    Some(TrainingProjection::Real) => "real",
                    Some(TrainingProjection::Imag) => "imag",
                    Some(TrainingProjection::AbsSq) => "norm_squared",
                    _ => "norm",
                })
            }
            (component, _) => component,
        };
        let index = match component {
            Some("norm") if self.pooled.is_some() => 2,
            Some("norm_squared") if self.pooled.is_some() => 3,
            None | Some("training_projection") => match &self.config {
                AccumulatorConfig::Vector {
                    training_projection: TrainingProjection::Norm,
                    ..
                } => self.plan.components.len(),
                AccumulatorConfig::Vector {
                    training_projection: TrainingProjection::Component { name },
                    ..
                } => self.plan.components.iter().position(|n| n == name)?,
                _ => return None,
            },
            Some(name) => self.plan.components.iter().position(|n| n == name)?,
        };
        let mean = self.mean()?[index];
        let error = self
            .covariance()
            .map(|c| c[index * self.strata[0].mean.len() + index].max(0.).sqrt());
        let value = match selector.name {
            AccumulatorMetricName::Mean => mean,
            AccumulatorMetricName::Error => error?,
            AccumulatorMetricName::RelativeError => super::relative_error(mean, error?),
            // Pooled sample variance/ESS/absolute moments do not describe this estimator.
            _ => return None,
        };
        if !value.is_finite() {
            return None;
        }
        Some(MeasurementResult {
            estimator: crate::core::EstimateKind::Stratified,
            basic: (selector.name == AccumulatorMetricName::Mean)
                .then(|| self.basic_estimates().map(|v| v[index].clone()))
                .flatten(),
            name: selector.name,
            component: Some(
                selector
                    .component
                    .clone()
                    .unwrap_or_else(|| "training_projection".into()),
            ),
            value,
            uncertainty: if selector.name == AccumulatorMetricName::Mean {
                error
            } else {
                None
            },
            sample_count: self.sample_count(),
        })
    }
    pub fn uncertainty_ready(&self) -> bool {
        self.strata.iter().all(StratumMoments::uncertainty_ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::StratificationConfig;

    #[test]
    fn pooled_reference_matches_direct_samples_through_merge_and_restore() {
        use crate::evaluation::ScalarAccumulatorState;
        let mut left = empty();
        let mut right = empty();
        let mut direct = [
            ScalarAccumulatorState::plain(),
            ScalarAccumulatorState::plain(),
        ];
        let probabilities = left.plan.probabilities.clone().unwrap();
        for i in 0..160 {
            let k = usize::from(i >= 64);
            let hit = i % 5 != 0;
            let value = if hit { (i % 7) as f64 - 2.0 } else { 0.0 };
            let values = [value, value * 3.0 + if hit { 1.0 } else { 0.0 }];
            let point = crate::Point::new(vec![i as f64], vec![], 1.0 / probabilities[k]);
            for (state, value) in direct.iter_mut().zip(values) {
                state.add_sample(value, &point).unwrap();
            }
            let target = if i < 91 { &mut left } else { &mut right };
            target.push(k, &values, point.total_weight(), hit).unwrap();
        }
        let saved = serde_json::to_value(left).unwrap();
        let mut merged: StratifiedAccumulatorState = serde_json::from_value(saved).unwrap();
        merged.merge(right).unwrap();
        let basic = merged.basic_estimates().unwrap();
        for (basic, direct) in basic.iter().zip(&direct) {
            assert!((basic.mean - direct.mean()).abs() < 1e-12);
            assert!((basic.error.unwrap() - direct.stderr().unwrap()).abs() < 1e-12);
        }
        let name = merged.plan.components[0].clone();
        let selected = merged
            .metric(&AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some(name),
            })
            .unwrap();
        assert_eq!(selected.estimator, crate::core::EstimateKind::Stratified);
        assert_eq!(selected.basic.as_ref().unwrap().mean, basic[0].mean);
        assert!((selected.value - basic[0].mean).abs() > 1e-3);
        assert_ne!(selected.uncertainty, basic[0].error);
        let state = super::super::AccumulatorState::Stratified(Box::new(merged));
        let stop = super::super::metrics::estimate_for_stop(
            &state,
            Some(&AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: selected.component.clone(),
            }),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(stop.error, selected.uncertainty.unwrap());
        assert_eq!(state.sample_count(), 160);
    }
    fn empty() -> StratifiedAccumulatorState {
        let plan = StratificationPlan {
            proposal_id: "fixture".into(),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![1., 2.], vec![-1., -2.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
                scales: None,
            },
            probabilities: Some(vec![0.25, 0.75]),
            components: vec!["x".into(), "y".into()],
        };
        StratifiedAccumulatorState::new(
            plan,
            AccumulatorConfig::vector(vec!["x".into(), "y".into()], TrainingProjection::Norm),
        )
        .unwrap()
    }
    #[test]
    fn insufficient_coverage_never_publishes_zero_uncertainty() {
        let mut state = empty();
        for _ in 0..64 {
            state.push(0, &[0., 0.], 4., false).unwrap();
            state.push(1, &[0., 0.], 4. / 3., false).unwrap();
        }
        assert!(state.covariance().is_none());
        assert!(!state.uncertainty_ready());
        let selector = AccumulatorMetricSelector {
            name: AccumulatorMetricName::Mean,
            component: Some("x".into()),
        };
        let mean = state.metric(&selector).unwrap();
        assert_eq!(mean.value, 0.0);
        assert_eq!(mean.uncertainty, None);
        assert_eq!(mean.basic.as_ref().unwrap().error, Some(0.0));
        let wrapped = super::super::AccumulatorState::Stratified(Box::new(state.clone()));
        assert!(
            super::super::metrics::estimate_for_stop(&wrapped, Some(&selector), None)
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .metric(&AccumulatorMetricSelector {
                    name: AccumulatorMetricName::Error,
                    ..selector.clone()
                })
                .is_none()
        );
        assert!(
            state
                .metric(&AccumulatorMetricSelector {
                    name: AccumulatorMetricName::RelativeError,
                    ..selector
                })
                .is_none()
        );
        assert_eq!(state.strata[0].standard_error(0), None);
        let restored: StratifiedAccumulatorState =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert!(restored.covariance().is_none());
        for _ in 0..2 {
            state.push(0, &[1., 2.], 4., true).unwrap();
            state.push(1, &[-1., -2.], 4. / 3., true).unwrap();
        }
        assert!(state.uncertainty_ready());
    }
    #[test]
    fn unequal_probabilities_misses_and_parallel_covariance_are_normalized_once() {
        let mut whole = empty();
        let mut merged = empty();
        assert!(whole.mean().is_none());
        assert!(whole.covariance().is_none());
        for (label, value, weight, matched) in [
            (0, [1., 2.], 4., true),
            (0, [0., 0.], 4., false),
            (1, [-1., -2.], 4. / 3., true),
            (1, [0., 0.], 4. / 3., false),
        ]
        .into_iter()
        .cycle()
        .take(64)
        {
            whole.push(label, &value, weight, matched).unwrap();
            let mut one = empty();
            one.push(label, &value, weight, matched).unwrap();
            merged.merge(one).unwrap();
        }
        assert_eq!(whole.sample_count(), 64);
        assert_eq!(whole.strata[0].matches, 16);
        assert_eq!(whole.mean(), merged.mean());
        for (left, right) in whole
            .covariance()
            .unwrap()
            .iter()
            .zip(merged.covariance().unwrap())
        {
            assert!((left - right).abs() < 1e-14);
        }
        let mean = whole.mean().unwrap();
        assert_eq!(&mean[..2], &[0., 0.]);
        let covariance = whole.covariance().unwrap();
        assert!((covariance[0] - 0.5 / 31.).abs() < 1e-14);
        assert!((covariance[1] - 1. / 31.).abs() < 1e-14);
        assert!((covariance[4] - 2. / 31.).abs() < 1e-14);
        let roundtrip: StratifiedAccumulatorState =
            serde_json::from_str(&serde_json::to_string(&whole).unwrap()).unwrap();
        roundtrip.validate().unwrap();
        for (left, right) in whole
            .covariance()
            .unwrap()
            .iter()
            .zip(roundtrip.covariance().unwrap())
        {
            assert!((left - right).abs() < 1e-14);
        }
        let mut incompatible = empty();
        incompatible.plan.probabilities = Some(vec![0.5, 0.5]);
        let before = serde_json::to_value(&whole).unwrap();
        assert!(whole.merge(incompatible).is_err());
        assert_eq!(serde_json::to_value(&whole).unwrap(), before);
    }
}

#[cfg(test)]
mod calibration_tests {
    use super::*;
    use crate::core::StratificationConfig;
    use rand::{Rng, SeedableRng};
    #[test]
    fn reported_variance_matches_repeated_independent_sampling_with_misses() {
        let mut rng = rand_xoshiro::Xoshiro256StarStar::seed_from_u64(81917);
        let p = [0.2, 0.8];
        let acceptance = 0.85;
        let plan = StratificationPlan {
            proposal_id: "calibration".into(),
            partition: StratificationConfig::Voronoi {
                domain: crate::Domain::strata(vec![2]),
                centers: vec![vec![-1.], vec![1.]]
                    .into_iter()
                    .enumerate()
                    .map(|(k, c)| (vec![k as i64], c))
                    .collect(),
                scales: None,
            },
            probabilities: Some(p.to_vec()),
            components: vec!["value".into()],
        };
        let mut means = Vec::new();
        let mut variances = Vec::new();
        for _ in 0..512 {
            let mut state =
                StratifiedAccumulatorState::new(plan.clone(), AccumulatorConfig::scalar()).unwrap();
            for _ in 0..2048 {
                let k = usize::from(rng.random::<f64>() >= p[0]);
                let matched = rng.random::<f64>() < acceptance;
                let value = if matched {
                    if k == 0 { -1. } else { 1. }
                } else {
                    0.
                };
                let conditional_density = 2. * if matched { acceptance } else { 1. - acceptance };
                state
                    .push(k, &[value], 1. / (p[k] * conditional_density), matched)
                    .unwrap();
            }
            means.push(state.mean().unwrap()[0]);
            variances.push(state.covariance().unwrap()[0]);
        }
        let mean = means.iter().sum::<f64>() / means.len() as f64;
        let empirical =
            means.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (means.len() - 1) as f64;
        let predicted = variances.iter().sum::<f64>() / variances.len() as f64;
        assert!(
            (0.75..1.25).contains(&(empirical / predicted)),
            "empirical {empirical}, predicted {predicted}"
        );
        assert!(
            mean.abs() < 4. * (empirical / means.len() as f64).sqrt(),
            "unexpected bias {mean}"
        );
    }
}
