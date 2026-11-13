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
    /// Absolute conditional-contribution sums. Older snapshots lack these;
    /// keep that absence explicit instead of inventing an ESS from variance.
    #[serde(default)]
    pub sum_abs: Option<Vec<f64>>,
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
            sum_abs: Some(vec![0.; width]),
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
        let sum_abs = self.sum_abs.as_ref().map(|sums| {
            sums.iter()
                .zip(values)
                .map(|(sum, value)| sum + value.abs())
                .collect::<Vec<_>>()
        });
        if sum_abs
            .as_ref()
            .is_some_and(|sums| sums.iter().any(|x| !x.is_finite()))
        {
            return Err(EngineError::eval("stratified absolute moment overflow"));
        }
        self.sum_abs = sum_abs;
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
        self.sum_abs = self
            .sum_abs
            .as_ref()
            .zip(other.sum_abs.as_ref())
            .map(|(left, right)| left.iter().zip(right).map(|(a, b)| a + b).collect());
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
    /// Pooled reference estimate and diagnostics.
    pub pooled: Option<Box<super::GammaLoopAccumulatorState>>,
    /// Conditional-weight raw statistics, indexed by the plan’s stratum order.
    #[cfg(feature = "gammaloop")]
    pub histograms: Vec<gammalooprs::observables::ObservableSnapshotBundle>,
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
            #[cfg(feature = "gammaloop")]
            histograms: Vec<gammalooprs::observables::ObservableSnapshotBundle>,
        }
        let saved = Saved::deserialize(deserializer)?;
        let state = Self {
            plan: saved.plan,
            config: saved.config,
            strata: saved.strata,
            pooled: saved.pooled,
            #[cfg(feature = "gammaloop")]
            histograms: saved.histograms,
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
            #[cfg(feature = "gammaloop")]
            histograms: Vec::new(),
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
                || s.sum_abs.as_ref().is_some_and(|sums| {
                    sums.len() != width || sums.iter().any(|v| !v.is_finite() || *v < 0.0)
                })
            {
                return Err(EngineError::eval("invalid stratum moments"));
            }
            count = count
                .checked_add(s.attempts)
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(|| EngineError::eval("stratum count overflow"))?;
        }
        #[cfg(feature = "gammaloop")]
        if !self.histograms.is_empty() {
            if !matches!(self.config, AccumulatorConfig::Gammaloop)
                || self.histograms.len() != self.strata.len()
            {
                return Err(EngineError::eval("incomplete stratified histogram state"));
            }
            gammalooprs::observables::ObservableSnapshotBundle::sum_independent_estimates(
                &self.histograms,
            )
            .map_err(|e| EngineError::eval(e.to_string()))?;
            for (bundle, stratum) in self.histograms.iter().zip(&self.strata) {
                super::gammaloop::validate_histogram_bundle(bundle)?;
                if bundle
                    .histograms
                    .values()
                    .any(|h| h.sample_count as u64 != stratum.attempts)
                {
                    return Err(EngineError::eval(
                        "histograms must count every intended-stratum attempt",
                    ));
                }
            }
        } else if matches!(self.config, AccumulatorConfig::Gammaloop)
            && count > 0
            && self
                .pooled
                .as_ref()
                .is_some_and(|p| !p.bundle.histograms.is_empty())
        {
            return Err(EngineError::eval(
                "missing conditional histogram statistics",
            ));
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
        #[cfg(feature = "gammaloop")]
        if merged.histograms.is_empty() {
            merged.histograms = other.histograms;
        } else if !other.histograms.is_empty() {
            for (left, right) in merged.histograms.iter_mut().zip(&other.histograms) {
                left.merge_in_place(right)
                    .map_err(|e| EngineError::eval(e.to_string()))?;
            }
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

    /// Integral of the absolute component using the same stratum assignment.
    /// Squaring removes the sign, so stored second moments also determine its
    /// error. Older checkpoints without absolute sums remain unavailable.
    fn absolute_estimate(&self, index: usize) -> Option<crate::core::MeanEstimate> {
        let mut mean = 0.0;
        let mut error = 0.0_f64;
        for stratum in &self.strata {
            if stratum.attempts == 0 {
                return None;
            }
            let n = stratum.attempts as f64;
            let absolute = stratum.sum_abs.as_ref()?.get(index)? / n;
            mean += absolute;
            if stratum.uncertainty_ready() {
                let signed = stratum.mean[index];
                let centered_second = stratum.m2[index * stratum.mean.len() + index] / n
                    + (signed - absolute) * (signed + absolute);
                error = error.hypot((centered_second.max(0.0) / (n - 1.0)).sqrt());
            }
        }
        Some(crate::core::MeanEstimate {
            mean,
            error: self.uncertainty_ready().then_some(error),
        })
    }

    /// Pooled reference from the same masked draws. Scale each conditional
    /// group back by 1/p, then merge central moments without subtracting raw
    /// squared sums. This is a different mean, not another error on `mean()`.
    fn pooled_moments(&self) -> Option<StratumMoments> {
        let width = self.strata[0].mean.len();
        let mut pooled = StratumMoments::new(width);
        for (stratum, &p) in self.strata.iter().zip(self.plan.probabilities.as_ref()?) {
            let mut group = StratumMoments::new(width);
            group.attempts = stratum.attempts;
            group.mean = stratum.mean.iter().map(|v| v / p).collect();
            group.m2 = stratum.m2.iter().map(|v| (v / p) / p).collect();
            // Absolute moments are not needed for the pooled reference estimate.
            group.sum_abs = None;
            if group.mean.iter().chain(&group.m2).any(|v| !v.is_finite()) {
                return None;
            }
            pooled.merge(&group).ok()?;
            if pooled.mean.iter().chain(&pooled.m2).any(|v| !v.is_finite()) {
                return None;
            }
        }
        Some(pooled)
    }

    pub fn pooled_estimates(&self) -> Option<Vec<crate::core::MeanEstimate>> {
        let pooled = self.pooled_moments()?;
        let width = pooled.mean.len();
        let n = pooled.attempts;
        if n == 0 {
            return None;
        }
        Some(
            pooled
                .mean
                .iter()
                .enumerate()
                .map(|(j, &mean)| crate::core::MeanEstimate {
                    mean,
                    error: (n >= 2).then(|| {
                        (pooled.m2[j * width + j].max(0.0) / (n as f64 * (n - 1) as f64)).sqrt()
                    }),
                })
                .collect(),
        )
    }

    fn training_projection_index(&self) -> Option<(usize, bool)> {
        if let Some(pooled) = &self.pooled {
            use crate::evaluation::evaluator::gammaloop::TrainingProjection;
            return Some(match pooled.training_projection {
                Some(TrainingProjection::Real) => (0, false),
                Some(TrainingProjection::Imag) => (1, false),
                Some(TrainingProjection::AbsSq) => (3, true),
                Some(TrainingProjection::Abs) | None => (2, true),
            });
        }
        match &self.config {
            AccumulatorConfig::Vector {
                training_projection: TrainingProjection::Norm,
                ..
            } => Some((self.plan.components.len(), true)),
            AccumulatorConfig::Vector {
                training_projection: TrainingProjection::Component { name },
                ..
            } => self
                .plan
                .components
                .iter()
                .position(|component| component == name)
                .map(|index| (index, false)),
            _ => None,
        }
    }

    /// Kish ESS/N within each assigned stratum, including zero-valued misses.
    /// Constant conditional-weight factors cancel inside each stratum. Mixing
    /// strata first would confound training quality with different allocations.
    pub fn training_ess_by_stratum(&self) -> Vec<Option<f64>> {
        let Some((index, nonnegative)) = self.training_projection_index() else {
            return vec![None; self.strata.len()];
        };
        self.strata
            .iter()
            .map(|stratum| {
                let n = stratum.attempts as f64;
                if n == 0.0 {
                    return None;
                }
                let mean = stratum.mean[index];
                let rms = mean
                    .hypot((stratum.m2[index * stratum.mean.len() + index].max(0.0) / n).sqrt());
                if rms == 0.0 {
                    return Some(0.0);
                }
                let mean_abs = if nonnegative {
                    mean.max(0.0)
                } else {
                    *stratum.sum_abs.as_ref()?.get(index)? / n
                };
                let ess = (mean_abs / rms).powi(2);
                ess.is_finite().then(|| ess.clamp(0.0, 1.0))
            })
            .collect()
    }

    /// Sum of the per-stratum effective counts, divided by all attempted draws.
    pub fn training_ess(&self) -> Option<f64> {
        let n = self.sample_count();
        if n == 0 {
            return None;
        }
        let mut fraction = 0.0;
        for (stratum, ess) in self.strata.iter().zip(self.training_ess_by_stratum()) {
            if stratum.attempts > 0 {
                fraction += stratum.attempts as f64 / n as f64 * ess?;
            }
        }
        Some(fraction.clamp(0.0, 1.0))
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
        let absolute = (selector.name == AccumulatorMetricName::AbsMean)
            .then(|| self.absolute_estimate(index))
            .flatten();
        let mean = if selector.name == AccumulatorMetricName::AbsMean {
            absolute.as_ref()?.mean
        } else {
            self.mean()?[index]
        };
        let error = if selector.name == AccumulatorMetricName::AbsMean {
            absolute?.error
        } else {
            self.covariance()
                .map(|c| c[index * self.strata[0].mean.len() + index].max(0.).sqrt())
        };
        let value = match selector.name {
            AccumulatorMetricName::Mean | AccumulatorMetricName::AbsMean => mean,
            AccumulatorMetricName::Error => error?,
            AccumulatorMetricName::RelativeError => super::relative_error(mean, error?),
            AccumulatorMetricName::Rsd => error? / mean.abs() * (self.sample_count() as f64).sqrt(),
            // Pooled sample variance/ESS do not describe this estimator.
            _ => return None,
        };
        if !value.is_finite() {
            return None;
        }
        Some(MeasurementResult {
            estimator: crate::core::EstimateKind::Stratified,
            pooled: (selector.name == AccumulatorMetricName::Mean)
                .then(|| self.pooled_estimates().map(|v| v[index].clone()))
                .flatten(),
            name: selector.name,
            component: Some(
                selector
                    .component
                    .clone()
                    .unwrap_or_else(|| "training_projection".into()),
            ),
            value,
            uncertainty: if matches!(
                selector.name,
                AccumulatorMetricName::Mean | AccumulatorMetricName::AbsMean
            ) {
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
    fn absolute_component_estimate_respects_strata_misses_merge_and_legacy_data() {
        let mut left = empty();
        let mut right = empty();
        let mut absolute_samples = [Vec::new(), Vec::new()];
        for i in 0..160 {
            let k = usize::from(i >= 64);
            let hit = i % 5 != 0;
            let value = if hit { (i % 7) as f64 - 3.0 } else { 0.0 };
            let weight = 2.0;
            absolute_samples[k]
                .push(value.abs() * weight * left.plan.probabilities.as_ref().unwrap()[k]);
            let state = if i < 91 { &mut left } else { &mut right };
            state.push(k, &[value, -value], weight, hit).unwrap();
        }
        left.merge(right).unwrap();
        let selector = AccumulatorMetricSelector {
            name: AccumulatorMetricName::AbsMean,
            component: Some("x".into()),
        };
        let estimate = left.metric(&selector).unwrap();
        let mut expected = 0.0;
        let mut variance = 0.0;
        for values in &absolute_samples {
            let n = values.len() as f64;
            let mean = values.iter().sum::<f64>() / n;
            expected += mean;
            variance += values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n * (n - 1.0));
        }
        assert!((estimate.value - expected).abs() < 1e-12);
        assert!((estimate.uncertainty.unwrap() - variance.sqrt()).abs() < 1e-12);
        assert!(estimate.value > left.mean().unwrap()[0].abs());
        let mut saved = serde_json::to_value(&left).unwrap();
        let restored: StratifiedAccumulatorState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(restored.metric(&selector).unwrap().value, estimate.value);
        saved["strata"][0]
            .as_object_mut()
            .unwrap()
            .remove("sum_abs");
        let legacy: StratifiedAccumulatorState = serde_json::from_value(saved).unwrap();
        assert!(legacy.metric(&selector).is_none());
        let mut insufficient = empty();
        insufficient.push(0, &[1., 0.], 4., true).unwrap();
        insufficient.push(1, &[-1., 0.], 4. / 3., true).unwrap();
        let provisional = insufficient.metric(&selector).unwrap();
        assert_eq!(provisional.value, 2.0);
        assert_eq!(provisional.uncertainty, None);
    }

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
        let pooled = merged.pooled_estimates().unwrap();
        for (pooled, direct) in pooled.iter().zip(&direct) {
            assert!((pooled.mean - direct.mean()).abs() < 1e-12);
            assert!((pooled.error.unwrap() - direct.stderr().unwrap()).abs() < 1e-12);
        }
        let name = merged.plan.components[0].clone();
        let selected = merged
            .metric(&AccumulatorMetricSelector {
                name: AccumulatorMetricName::Mean,
                component: Some(name),
            })
            .unwrap();
        assert_eq!(selected.estimator, crate::core::EstimateKind::Stratified);
        assert_eq!(selected.pooled.as_ref().unwrap().mean, pooled[0].mean);
        assert!((selected.value - pooled[0].mean).abs() > 1e-3);
        assert_ne!(selected.uncertainty, pooled[0].error);
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
    #[test]
    fn signed_training_ess_respects_assignments_and_survives_merging() {
        let plan = empty().plan;
        let config =
            AccumulatorConfig::vector(plan.components.clone(), TrainingProjection::component("x"));
        let mut left = StratifiedAccumulatorState::new(plan.clone(), config.clone()).unwrap();
        let mut right = StratifiedAccumulatorState::new(plan, config).unwrap();
        for i in 0..160 {
            let k = usize::from(i >= 64);
            let value = if k == 0 {
                if i % 2 == 0 { -1.0 } else { 3.0 }
            } else if i % 2 == 0 {
                0.0
            } else {
                6.0
            };
            let state = if i < 70 { &mut left } else { &mut right };
            let p = state.plan.probabilities.as_ref().unwrap()[k];
            state
                .push(k, &[value, value * 2.0], 1.0 / p, value != 0.0)
                .unwrap();
        }
        let mut restored: StratifiedAccumulatorState =
            serde_json::from_value(serde_json::to_value(left).unwrap()).unwrap();
        restored.merge(right).unwrap();
        let ess = restored.training_ess_by_stratum();
        assert!((ess[0].unwrap() - 0.8).abs() < 1e-12);
        assert!((ess[1].unwrap() - 0.5).abs() < 1e-12);
        assert!((restored.training_ess().unwrap() - 0.62).abs() < 1e-12);
        let metric = restored
            .metric(&AccumulatorMetricSelector {
                name: AccumulatorMetricName::Rsd,
                component: Some("x".into()),
            })
            .unwrap();
        let expected = ((4.0 / 63.0 + 9.0 / 95.0) * 160.0_f64).sqrt() / 4.0;
        assert!((metric.value - expected).abs() < 1e-12);
        assert_eq!(metric.estimator, crate::core::EstimateKind::Stratified);
        let mut old = serde_json::to_value(&restored).unwrap();
        for stratum in old["strata"].as_array_mut().unwrap() {
            stratum.as_object_mut().unwrap().remove("sum_abs");
        }
        let mut old: StratifiedAccumulatorState = serde_json::from_value(old).unwrap();
        assert!(old.training_ess().is_none());
        assert_eq!(old.mean(), restored.mean());
        assert_eq!(old.covariance(), restored.covariance());
        old.merge(restored).unwrap();
        assert!(
            old.training_ess().is_none(),
            "new moments cannot repair missing historical absolute sums"
        );
    }

    #[test]
    fn nonnegative_training_ess_can_use_older_conditional_moments() {
        let mut state = empty();
        for k in 0..2 {
            for _ in 0..32 {
                state.push(k, &[-2.0, 3.0], 4.0, true).unwrap();
            }
        }
        let mut old = serde_json::to_value(&state).unwrap();
        for stratum in old["strata"].as_array_mut().unwrap() {
            stratum.as_object_mut().unwrap().remove("sum_abs");
        }
        let old: StratifiedAccumulatorState = serde_json::from_value(old).unwrap();
        assert!((old.training_ess().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn conditional_training_ess_avoids_squaring_large_or_tiny_means() {
        for scale in [1e-160, 1.0, 1e160] {
            let mut state = empty();
            for k in 0..2 {
                for _ in 0..32 {
                    state.push(k, &[scale, 0.0], 1.0, true).unwrap();
                }
            }
            state.validate().unwrap();
            assert!((state.training_ess().unwrap() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn native_ess_follows_each_training_projection_without_pooling_strata() {
        use crate::evaluation::evaluator::gammaloop::TrainingProjection as Phase;
        let mut plan = empty().plan;
        plan.components = vec!["real".into(), "imag".into()];
        let mut state =
            StratifiedAccumulatorState::new(plan, AccumulatorConfig::Gammaloop).unwrap();
        for i in 0..32 {
            state
                .push(
                    0,
                    &if i % 2 == 0 { [3.0, 4.0] } else { [-12.0, 5.0] },
                    4.0,
                    true,
                )
                .unwrap();
        }
        for _ in 0..96 {
            state.push(1, &[3.0, 4.0], 4.0 / 3.0, true).unwrap();
        }
        for (phase, expected) in [
            (Phase::Real, 56.25 / 76.5),
            (Phase::Imag, 20.25 / 20.5),
            (Phase::Abs, 81.0 / 97.0),
            (Phase::AbsSq, 9409.0 / 14593.0),
        ] {
            state.pooled.as_mut().unwrap().training_projection = Some(phase);
            let ess = state.training_ess_by_stratum();
            assert!((ess[0].unwrap() - expected).abs() < 1e-12);
            assert!((ess[1].unwrap() - 1.0).abs() < 1e-12);
            assert!((state.training_ess().unwrap() - (expected * 0.25 + 0.75)).abs() < 1e-12);
        }
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
        assert_eq!(mean.pooled.as_ref().unwrap().error, Some(0.0));
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
