use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::core::{
    AccumulatorConfig, BatchTransformConfig, BuildError, EvaluatorConfig, MeasurementResult,
    SamplerAggregatorConfig,
};
use crate::sampling::{RasterLineSamplerParams, RasterPlaneSamplerParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTaskState {
    Pending,
    Active,
    Completed,
    Failed,
}

impl RunTaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleTaskConfig {
    pub batch_transforms: Option<Vec<BatchTransformConfig>>,
}

pub const DEFAULT_DISCRETE_PROJECTION_MAX_TOTAL_BINS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DiscreteProjectionConfig {
    pub max_total_bins: Option<usize>,
    pub normalization: DiscreteProjectionNormalization,
    pub streams: Vec<String>,
    pub items: Vec<NamedDiscreteProjection>,
}

impl Default for DiscreteProjectionConfig {
    fn default() -> Self {
        Self {
            max_total_bins: None,
            normalization: DiscreteProjectionNormalization::Contribution,
            streams: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl DiscreteProjectionConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(limit) = self.max_total_bins
            && limit == 0
        {
            return Err(
                "accumulator.discrete_projections.max_total_bins must be > 0 when set".to_string(),
            );
        }
        if self.items.is_empty() {
            return Err(
                "accumulator.discrete_projections.items must contain at least one entry"
                    .to_string(),
            );
        }
        let mut names = BTreeSet::new();
        let mut streams = BTreeSet::new();
        for stream in &self.streams {
            let trimmed = stream.trim();
            if trimmed.is_empty() {
                return Err(
                    "accumulator.discrete_projections.streams must not contain empty names"
                        .to_string(),
                );
            }
            if trimmed != stream {
                return Err(
                    "accumulator.discrete_projections.streams entries cannot have leading/trailing whitespace"
                        .to_string(),
                );
            }
            if !streams.insert(stream.clone()) {
                return Err(format!(
                    "accumulator.discrete_projections.streams contains duplicate name '{stream}'"
                ));
            }
        }
        for item in &self.items {
            item.validate()?;
            if !names.insert(item.name.clone()) {
                return Err(format!(
                    "accumulator.discrete_projections.items contains duplicate name '{}'",
                    item.name
                ));
            }
        }
        Ok(())
    }

    pub fn max_total_bins_or_default(&self) -> usize {
        self.max_total_bins
            .unwrap_or(DEFAULT_DISCRETE_PROJECTION_MAX_TOTAL_BINS)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiscreteProjectionNormalization {
    #[default]
    Contribution,
    ConditionalMean,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NamedDiscreteProjection {
    pub name: String,
    pub dims: Vec<usize>,
    pub fixed_dims: BTreeMap<String, i64>,
}

impl Default for NamedDiscreteProjection {
    fn default() -> Self {
        Self {
            name: String::new(),
            dims: Vec::new(),
            fixed_dims: BTreeMap::new(),
        }
    }
}

impl NamedDiscreteProjection {
    pub fn validate(&self) -> Result<(), String> {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            return Err(
                "accumulator.discrete_projections.items.name must be non-empty".to_string(),
            );
        }
        if trimmed != self.name {
            return Err(
                "accumulator.discrete_projections.items.name cannot have leading/trailing whitespace"
                    .to_string(),
            );
        }
        if self.dims.is_empty() {
            return Err(
                "accumulator.discrete_projections.items.dims must contain at least one dimension"
                    .to_string(),
            );
        }
        let mut seen = BTreeSet::new();
        for dim in &self.dims {
            if !seen.insert(*dim) {
                return Err(format!(
                    "accumulator.discrete_projections.items '{}' repeats dims entry {}",
                    self.name, dim
                ));
            }
        }
        for raw_dim in self.fixed_dims.keys() {
            let dim = raw_dim.parse::<usize>().map_err(|_| {
                format!(
                    "accumulator.discrete_projections.items '{}' fixed_dims key '{}' is not a non-negative integer dimension index",
                    self.name, raw_dim
                )
            })?;
            if seen.contains(&dim) {
                return Err(format!(
                    "accumulator.discrete_projections.items '{}' uses dimension {} in both dims and fixed_dims",
                    self.name, dim
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SampleErrorProjection {
    Real,
    Imag,
    Abs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccumulatorMetricName {
    Mean,
    AbsMean,
    Error,
    RelativeError,
    Variance,
    RelativeVarianceError,
    RelativeSquaredDispersion,
    TimeNormalizedVariance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccumulatorMetricSelector {
    pub name: AccumulatorMetricName,
    #[serde(default)]
    pub component: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MeasurementMetricSpec {
    Name(AccumulatorMetricName),
    Selector(AccumulatorMetricSelector),
}

impl MeasurementMetricSpec {
    pub fn selector(&self) -> AccumulatorMetricSelector {
        match self {
            Self::Name(name) => AccumulatorMetricSelector {
                name: *name,
                component: None,
            },
            Self::Selector(selector) => selector.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementQuantityName {
    #[default]
    CentralValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MeasurementMetricQuantity {
    pub metric: AccumulatorMetricName,
    #[serde(default)]
    pub component: Option<String>,
}

impl MeasurementMetricQuantity {
    pub fn selector(&self) -> AccumulatorMetricSelector {
        AccumulatorMetricSelector {
            name: self.metric,
            component: self.component.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MeasurementQuantitySpec {
    Name(MeasurementQuantityName),
    Metric(MeasurementMetricQuantity),
}

impl Default for MeasurementQuantitySpec {
    fn default() -> Self {
        Self::Name(MeasurementQuantityName::CentralValue)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementMode {
    #[default]
    Minimize,
    Maximize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TaskMeasurementSpec {
    pub quantity: MeasurementQuantitySpec,
    pub metric: Option<MeasurementMetricSpec>,
    #[serde(default)]
    pub mode: MeasurementMode,
}

impl Default for TaskMeasurementSpec {
    fn default() -> Self {
        Self {
            quantity: MeasurementQuantitySpec::default(),
            metric: None,
            mode: MeasurementMode::default(),
        }
    }
}

impl TaskMeasurementSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.metric.is_some()
            && self.quantity != MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue)
        {
            return Err("measurement cannot set both metric and quantity".to_string());
        }
        Ok(())
    }

    pub fn metric_selector(&self) -> AccumulatorMetricSelector {
        match self.metric.as_ref() {
            Some(metric) => metric.selector(),
            None => match &self.quantity {
                MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue) => {
                    AccumulatorMetricSelector {
                        name: AccumulatorMetricName::Mean,
                        component: None,
                    }
                }
                MeasurementQuantitySpec::Metric(metric) => metric.selector(),
            },
        }
    }

    pub fn explicit_metric_selector(&self) -> Option<AccumulatorMetricSelector> {
        match self.metric.as_ref() {
            Some(metric) => Some(metric.selector()),
            None => match &self.quantity {
                MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue) => None,
                MeasurementQuantitySpec::Metric(metric) => Some(metric.selector()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MeasurementSpec {
    pub source_task: String,
    pub quantity: MeasurementQuantitySpec,
    pub metric: Option<MeasurementMetricSpec>,
    #[serde(default)]
    pub mode: MeasurementMode,
}

impl Default for MeasurementSpec {
    fn default() -> Self {
        Self {
            source_task: String::new(),
            quantity: MeasurementQuantitySpec::default(),
            metric: None,
            mode: MeasurementMode::default(),
        }
    }
}

impl MeasurementSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_source_name("measurement.source_task", &self.source_task)?;
        self.task_measurement().validate()
    }

    pub fn task_measurement(&self) -> TaskMeasurementSpec {
        TaskMeasurementSpec {
            quantity: self.quantity.clone(),
            metric: self.metric.clone(),
            mode: self.mode,
        }
    }

    pub fn metric_selector(&self) -> AccumulatorMetricSelector {
        self.task_measurement().metric_selector()
    }

    pub fn explicit_metric_selector(&self) -> Option<AccumulatorMetricSelector> {
        self.task_measurement().explicit_metric_selector()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParameterScanParameterSpec {
    pub name: String,
    pub values: Vec<toml::Value>,
}

impl ParameterScanParameterSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_source_name("parameter_scan.parameters", &self.name)?;
        if self.values.is_empty() {
            return Err(format!(
                "parameter_scan.parameters.{}.values must not be empty",
                self.name
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ParameterScanMeasurementSpec {
    pub source_task: String,
}

impl Default for ParameterScanMeasurementSpec {
    fn default() -> Self {
        Self {
            source_task: "sample".to_string(),
        }
    }
}

impl ParameterScanMeasurementSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_source_name("measurement.source_task", &self.source_task)
    }
}

pub fn effective_parameter_scan_parameters(
    parameter: &Option<ParameterScanParameterSpec>,
    parameters: &[ParameterScanParameterSpec],
) -> Result<Vec<ParameterScanParameterSpec>, String> {
    match (parameter, parameters.is_empty()) {
        (Some(_), false) => Err(
            "parameter_scan must use either [parameter] or [[parameters]], not both".to_string(),
        ),
        (Some(parameter), true) => Ok(vec![parameter.clone()]),
        (None, false) => Ok(parameters.to_vec()),
        (None, true) => Err("parameter_scan.parameters must not be empty".to_string()),
    }
}

pub fn parameter_scan_grid_len(
    parameter: &Option<ParameterScanParameterSpec>,
    parameters: &[ParameterScanParameterSpec],
) -> Option<usize> {
    let parameters = effective_parameter_scan_parameters(parameter, parameters).ok()?;
    parameters.iter().try_fold(1usize, |count, parameter| {
        count.checked_mul(parameter.values.len())
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HyperparameterTuningFloatDomain {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HyperparameterTuningIntegerDomain {
    pub min: i64,
    pub max: i64,
    #[serde(default)]
    pub step: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HyperparameterTuningCategoricalDomain {
    pub values: Vec<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HyperparameterTuningParameterDomain {
    Float(HyperparameterTuningFloatDomain),
    Integer(HyperparameterTuningIntegerDomain),
    Categorical(HyperparameterTuningCategoricalDomain),
}

impl HyperparameterTuningParameterDomain {
    pub fn validate(&self, name: &str) -> Result<(), String> {
        validate_source_name("hyperparameter_tuning.parameters", name)?;
        match self {
            Self::Float(domain) => {
                if !domain.min.is_finite() || !domain.max.is_finite() {
                    return Err(format!(
                        "hyperparameter_tuning.parameters.{name} float bounds must be finite"
                    ));
                }
                if domain.min >= domain.max {
                    return Err(format!(
                        "hyperparameter_tuning.parameters.{name} float min must be < max"
                    ));
                }
                Ok(())
            }
            Self::Integer(domain) => {
                if domain.min > domain.max {
                    return Err(format!(
                        "hyperparameter_tuning.parameters.{name} integer min must be <= max"
                    ));
                }
                if domain.step.is_some_and(|step| step <= 0) {
                    return Err(format!(
                        "hyperparameter_tuning.parameters.{name} integer step must be > 0"
                    ));
                }
                Ok(())
            }
            Self::Categorical(domain) => {
                if domain.values.is_empty() {
                    return Err(format!(
                        "hyperparameter_tuning.parameters.{name} categorical values must not be empty"
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HyperparameterTuningAlgorithm {
    RandomSearch,
    GridSearch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HyperparameterTuningOptimizerSpec {
    pub algorithm: HyperparameterTuningAlgorithm,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default = "empty_json_object")]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RandomSearchOptimizerParams {
    pub max_trials: usize,
}

fn empty_json_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

impl HyperparameterTuningOptimizerSpec {
    pub fn validate(&self) -> Result<(), String> {
        let Some(params) = self.params.as_object() else {
            return Err("hyperparameter_tuning.optimizer.params must be a table".to_string());
        };
        match self.algorithm {
            HyperparameterTuningAlgorithm::GridSearch if !params.is_empty() => Err(
                "hyperparameter_tuning.optimizer.params must be empty for grid_search".to_string(),
            ),
            HyperparameterTuningAlgorithm::RandomSearch => {
                let params = self.random_search_params()?;
                if params.max_trials == 0 {
                    return Err(
                        "hyperparameter_tuning.optimizer.params.max_trials must be > 0".to_string(),
                    );
                }
                Ok(())
            }
            HyperparameterTuningAlgorithm::GridSearch => Ok(()),
        }
    }

    pub fn random_search_params(&self) -> Result<RandomSearchOptimizerParams, String> {
        serde_json::from_value(self.params.clone()).map_err(|err| {
            format!("invalid hyperparameter_tuning.optimizer.params for random_search: {err}")
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HyperparameterTuningObjectiveSpec {
    pub source_task: String,
    pub quantity: MeasurementQuantitySpec,
    pub metric: Option<MeasurementMetricSpec>,
    #[serde(default)]
    pub mode: MeasurementMode,
}

impl Default for HyperparameterTuningObjectiveSpec {
    fn default() -> Self {
        Self {
            source_task: "sample".to_string(),
            quantity: MeasurementQuantitySpec::default(),
            metric: None,
            mode: MeasurementMode::default(),
        }
    }
}

impl HyperparameterTuningObjectiveSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_source_name(
            "hyperparameter_tuning.objective.source_task",
            &self.source_task,
        )?;
        TaskMeasurementSpec {
            quantity: self.quantity.clone(),
            metric: self.metric.clone(),
            mode: self.mode,
        }
        .validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SampleStopCondition {
    pub min_samples: Option<i64>,
    pub max_samples: Option<i64>,
    pub absolute_error: Option<f64>,
    pub relative_error: Option<f64>,
    pub projection: Option<SampleErrorProjection>,
    pub metric: Option<AccumulatorMetricSelector>,
}

impl Default for SampleStopCondition {
    fn default() -> Self {
        Self {
            min_samples: None,
            max_samples: None,
            absolute_error: None,
            relative_error: None,
            projection: None,
            metric: None,
        }
    }
}

impl SampleStopCondition {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_samples.is_some_and(|value| value < 0) {
            return Err(
                "sample.stop_condition.min_samples must be a non-negative integer when set"
                    .to_string(),
            );
        }
        if self.max_samples.is_some_and(|value| value < 0) {
            return Err(
                "sample.stop_condition.max_samples must be a non-negative integer when set"
                    .to_string(),
            );
        }
        if let (Some(min), Some(max)) = (self.min_samples, self.max_samples)
            && min > max
        {
            return Err(
                "sample.stop_condition.min_samples must be <= max_samples when both are set"
                    .to_string(),
            );
        }
        if let Some(value) = self.absolute_error
            && (!value.is_finite() || value <= 0.0)
        {
            return Err(
                "sample.stop_condition.absolute_error must be finite and > 0 when set".to_string(),
            );
        }
        if let Some(value) = self.relative_error
            && (!value.is_finite() || value <= 0.0)
        {
            return Err(
                "sample.stop_condition.relative_error must be finite and > 0 when set".to_string(),
            );
        }
        if self.max_samples.is_none()
            && self.absolute_error.is_none()
            && self.relative_error.is_none()
        {
            return Err(
                "sample.stop_condition must set at least one of: max_samples, absolute_error, relative_error"
                    .to_string(),
            );
        }
        if self.metric.is_some() && self.projection.is_some() {
            return Err(
                "sample.stop_condition.metric and projection are mutually exclusive".to_string(),
            );
        }
        if self.projection.is_some()
            && self.absolute_error.is_none()
            && self.relative_error.is_none()
        {
            return Err(
                "sample.stop_condition.projection requires absolute_error and/or relative_error"
                    .to_string(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SamplerQueueTuning {
    pub queue_buffer: Option<f64>,
    pub target_batch_eval_ms: Option<f64>,
    pub batch_size_deadband_ratio: Option<f64>,
    pub batch_size_cooldown_ticks: Option<u32>,
    pub pending_refill_low_ratio: Option<f64>,
    pub pending_refill_high_ratio: Option<f64>,
    pub max_batch_size: Option<usize>,
    pub local_pending_buffer_multiplier: Option<f64>,
    pub max_queue_size: Option<usize>,
    pub max_batches_per_tick: Option<usize>,
    pub max_insert_bundle_size: Option<usize>,
    pub max_concurrent_insert_tasks: Option<usize>,
    pub completed_batch_fetch_limit: Option<usize>,
}

impl Default for SamplerQueueTuning {
    fn default() -> Self {
        Self {
            queue_buffer: None,
            target_batch_eval_ms: None,
            batch_size_deadband_ratio: None,
            batch_size_cooldown_ticks: None,
            pending_refill_low_ratio: None,
            pending_refill_high_ratio: None,
            max_batch_size: None,
            local_pending_buffer_multiplier: None,
            max_queue_size: None,
            max_batches_per_tick: None,
            max_insert_bundle_size: None,
            max_concurrent_insert_tasks: None,
            completed_batch_fetch_limit: None,
        }
    }
}

impl SamplerQueueTuning {
    pub fn validate(&self) -> Result<(), String> {
        fn validate_non_negative_finite(value: Option<f64>, label: &str) -> Result<(), String> {
            if let Some(value) = value
                && (!value.is_finite() || value < 0.0)
            {
                return Err(format!("{label} must be finite and >= 0"));
            }
            Ok(())
        }

        fn validate_positive(value: Option<usize>, label: &str) -> Result<(), String> {
            if value.is_some_and(|value| value == 0) {
                return Err(format!("{label} must be > 0"));
            }
            Ok(())
        }

        validate_non_negative_finite(self.queue_buffer, "queue_tuning.queue_buffer")?;
        if let Some(value) = self.target_batch_eval_ms
            && (!value.is_finite() || value <= 0.0)
        {
            return Err("queue_tuning.target_batch_eval_ms must be finite and > 0".to_string());
        }
        validate_non_negative_finite(
            self.batch_size_deadband_ratio,
            "queue_tuning.batch_size_deadband_ratio",
        )?;
        validate_non_negative_finite(
            self.pending_refill_low_ratio,
            "queue_tuning.pending_refill_low_ratio",
        )?;
        validate_non_negative_finite(
            self.pending_refill_high_ratio,
            "queue_tuning.pending_refill_high_ratio",
        )?;
        validate_non_negative_finite(
            self.local_pending_buffer_multiplier,
            "queue_tuning.local_pending_buffer_multiplier",
        )?;
        validate_positive(self.max_batch_size, "queue_tuning.max_batch_size")?;
        validate_positive(self.max_queue_size, "queue_tuning.max_queue_size")?;
        validate_positive(
            self.max_batches_per_tick,
            "queue_tuning.max_batches_per_tick",
        )?;
        validate_positive(
            self.max_insert_bundle_size,
            "queue_tuning.max_insert_bundle_size",
        )?;
        validate_positive(
            self.max_concurrent_insert_tasks,
            "queue_tuning.max_concurrent_insert_tasks",
        )?;
        validate_positive(
            self.completed_batch_fetch_limit,
            "queue_tuning.completed_batch_fetch_limit",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefSpec {
    Latest,
    FromName(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SamplerAggregatorSourceSpec {
    Latest(String),
    FromName { from_name: String },
    Config { config: SamplerAggregatorConfig },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EvaluatorSourceSpec {
    Latest(String),
    FromName { from_name: String },
    Config { config: EvaluatorConfig },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AccumulatorSourceSpec {
    Latest(String),
    FromName { from_name: String },
    Config { config: AccumulatorConfig },
}

fn validate_source_name(field: &str, from_name: &str) -> Result<(), String> {
    let trimmed = from_name.trim();
    if trimmed.is_empty() {
        return Err(format!("{field}.from_name must be non-empty"));
    }
    if trimmed != from_name {
        return Err(format!(
            "{field}.from_name cannot have leading/trailing whitespace"
        ));
    }
    Ok(())
}

impl SamplerAggregatorSourceSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Latest(value) => {
                if value == "latest" {
                    Ok(())
                } else {
                    Err("sampler_aggregator must be one of: \"latest\", { from_name = ... }, { config = ... }".to_string())
                }
            }
            Self::FromName { from_name } => validate_source_name("sampler_aggregator", from_name),
            Self::Config { .. } => Ok(()),
        }
    }
}

impl EvaluatorSourceSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Latest(value) => {
                if value == "latest" {
                    Ok(())
                } else {
                    Err("evaluator must be one of: \"latest\", { from_name = ... }, { config = ... }".to_string())
                }
            }
            Self::FromName { from_name } => validate_source_name("evaluator", from_name),
            Self::Config { .. } => Ok(()),
        }
    }
}

impl AccumulatorSourceSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Latest(value) => {
                if value == "latest" {
                    Ok(())
                } else {
                    Err("accumulator must be one of: \"latest\", { from_name = ... }, { config = ... }".to_string())
                }
            }
            Self::FromName { from_name } => validate_source_name("accumulator", from_name),
            Self::Config { config } => config.validate(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTaskSpec {
    SetAccumulator {
        accumulator: AccumulatorConfig,
    },
    Sample {
        stop_condition: SampleStopCondition,
        #[serde(default)]
        measurement: Option<TaskMeasurementSpec>,
        #[serde(default)]
        evaluator: Option<EvaluatorSourceSpec>,
        #[serde(default)]
        sampler_aggregator: Option<SamplerAggregatorSourceSpec>,
        #[serde(default)]
        accumulator: Option<AccumulatorSourceSpec>,
        #[serde(default)]
        queue_tuning: Option<SamplerQueueTuning>,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    Image {
        geometry: PlaneRasterGeometry,
        accumulator: PlotAccumulatorKind,
        #[serde(default)]
        evaluator: Option<EvaluatorSourceSpec>,
        #[serde(default)]
        display: ImageDisplayMode,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    PdfAdaptationImage {
        geometry: PlaneRasterGeometry,
        #[serde(default)]
        sampler_aggregator: Option<SamplerAggregatorSourceSpec>,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    PdfAdaptationPlotLine {
        geometry: LineRasterGeometry,
        #[serde(default)]
        sampler_aggregator: Option<SamplerAggregatorSourceSpec>,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    PlotLine {
        geometry: LineRasterGeometry,
        accumulator: PlotAccumulatorKind,
        #[serde(default)]
        evaluator: Option<EvaluatorSourceSpec>,
        #[serde(default)]
        display: LineDisplayMode,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    ParameterScan {
        #[serde(default)]
        parameter: Option<ParameterScanParameterSpec>,
        #[serde(default)]
        parameters: Vec<ParameterScanParameterSpec>,
        #[serde(default)]
        measurement: ParameterScanMeasurementSpec,
        trial_run_toml: String,
        #[serde(default = "default_parameter_scan_max_concurrent_runs")]
        max_concurrent_runs: usize,
    },
    HyperparameterTuning {
        optimizer: HyperparameterTuningOptimizerSpec,
        objective: HyperparameterTuningObjectiveSpec,
        parameters: BTreeMap<String, HyperparameterTuningParameterDomain>,
        trial_run_toml: String,
        #[serde(default = "default_hyperparameter_tuning_max_concurrent_trials")]
        max_concurrent_trials: usize,
    },
}

fn default_parameter_scan_max_concurrent_runs() -> usize {
    1
}

fn default_hyperparameter_tuning_max_concurrent_trials() -> usize {
    1
}

impl RunTaskSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::SetAccumulator { accumulator } => accumulator.validate(),
            Self::Sample {
                stop_condition,
                sampler_aggregator:
                    Some(SamplerAggregatorSourceSpec::Config {
                        config: SamplerAggregatorConfig::HavanaTraining { .. },
                    }),
                ..
            } if stop_condition.max_samples.is_none() => Err(
                "sample task with havana_training sampler requires stop_condition.max_samples for training budget"
                    .to_string(),
            ),
            Self::Sample {
                stop_condition,
                measurement,
                evaluator,
                sampler_aggregator,
                accumulator,
                queue_tuning,
                ..
            } => {
                stop_condition.validate()?;
                if let Some(measurement) = measurement {
                    measurement.validate()?;
                }
                if let Some(source) = evaluator {
                    source.validate()?;
                }
                if let Some(source) = sampler_aggregator {
                    source.validate()?;
                }
                if let Some(source) = accumulator {
                    source.validate()?;
                }
                if let Some(queue_tuning) = queue_tuning {
                    queue_tuning.validate()?;
                }
                Ok(())
            }
            Self::Image {
                geometry, evaluator, ..
            } => {
                geometry.validate()?;
                if let Some(source) = evaluator {
                    source.validate()?;
                }
                Ok(())
            }
            Self::PdfAdaptationImage {
                geometry,
                sampler_aggregator,
                ..
            } => {
                geometry.validate()?;
                if let Some(source) = sampler_aggregator {
                    match source {
                        SamplerAggregatorSourceSpec::Config { .. } => {
                            return Err(
                                "pdf_adaptation_image sampler_aggregator must be omitted, \"latest\", or { from_name = ... }"
                                    .to_string(),
                            );
                        }
                        _ => source.validate()?,
                    }
                }
                Ok(())
            }
            Self::PdfAdaptationPlotLine {
                geometry,
                sampler_aggregator,
                ..
            } => {
                geometry.validate()?;
                if let Some(source) = sampler_aggregator {
                    match source {
                        SamplerAggregatorSourceSpec::Config { .. } => {
                            return Err(
                                "pdf_adaptation_plot_line sampler_aggregator must be omitted, \"latest\", or { from_name = ... }"
                                    .to_string(),
                            );
                        }
                        _ => source.validate()?,
                    }
                }
                Ok(())
            }
            Self::PlotLine {
                geometry, evaluator, ..
            } => {
                geometry.validate()?;
                if let Some(source) = evaluator {
                    source.validate()?;
                }
                Ok(())
            }
            Self::ParameterScan {
                parameter,
                parameters,
                measurement,
                trial_run_toml,
                max_concurrent_runs,
            } => {
                let parameters = effective_parameter_scan_parameters(parameter, parameters)?;
                for parameter in &parameters {
                    parameter.validate()?;
                }
                measurement.validate()?;
                if trial_run_toml.trim().is_empty() {
                    return Err("parameter_scan.trial_run_toml must be non-empty".to_string());
                }
                if *max_concurrent_runs == 0 {
                    return Err("parameter_scan.max_concurrent_runs must be > 0".to_string());
                }
                Ok(())
            }
            Self::HyperparameterTuning {
                optimizer,
                objective,
                parameters,
                trial_run_toml,
                max_concurrent_trials,
            } => {
                optimizer.validate()?;
                objective.validate()?;
                if parameters.is_empty() {
                    return Err(
                        "hyperparameter_tuning.parameters must not be empty".to_string()
                    );
                }
                for (name, domain) in parameters {
                    domain.validate(name)?;
                }
                if trial_run_toml.trim().is_empty() {
                    return Err(
                        "hyperparameter_tuning.trial_run_toml must be non-empty".to_string()
                    );
                }
                if *max_concurrent_trials == 0 {
                    return Err(
                        "hyperparameter_tuning.max_concurrent_trials must be > 0".to_string()
                    );
                }
                Ok(())
            }
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::SetAccumulator { .. } => "set_accumulator",
            Self::Sample { .. } => "sample",
            Self::Image { .. } => "image",
            Self::PdfAdaptationImage { .. } => "pdf_adaptation_image",
            Self::PdfAdaptationPlotLine { .. } => "pdf_adaptation_plot_line",
            Self::PlotLine { .. } => "plot_line",
            Self::ParameterScan { .. } => "parameter_scan",
            Self::HyperparameterTuning { .. } => "hyperparameter_tuning",
        }
    }

    pub fn runs_in_control_plane(&self) -> bool {
        matches!(
            self,
            Self::SetAccumulator { .. }
                | Self::ParameterScan { .. }
                | Self::HyperparameterTuning { .. }
        )
    }

    pub fn runs_on_sampler_worker(&self) -> bool {
        matches!(
            self,
            Self::Sample { .. }
                | Self::Image { .. }
                | Self::PdfAdaptationImage { .. }
                | Self::PdfAdaptationPlotLine { .. }
                | Self::PlotLine { .. }
        )
    }

    pub fn sampler_config(&self) -> Option<SamplerAggregatorConfig> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample { .. } => None,
            Self::Image { geometry, .. } => Some(SamplerAggregatorConfig::RasterPlane {
                params: RasterPlaneSamplerParams {
                    geometry: geometry.clone(),
                },
            }),
            Self::PdfAdaptationImage { geometry, .. } => {
                Some(SamplerAggregatorConfig::PdfAdaptationRasterPlane {
                    params: RasterPlaneSamplerParams {
                        geometry: geometry.clone(),
                    },
                })
            }
            Self::PdfAdaptationPlotLine { geometry, .. } => {
                Some(SamplerAggregatorConfig::PdfAdaptationRasterLine {
                    params: RasterLineSamplerParams {
                        geometry: geometry.clone(),
                    },
                })
            }
            Self::PlotLine { geometry, .. } => Some(SamplerAggregatorConfig::RasterLine {
                params: RasterLineSamplerParams {
                    geometry: geometry.clone(),
                },
            }),
            Self::ParameterScan { .. } | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn sample_sampler_source(&self) -> Option<SourceRefSpec> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample {
                sampler_aggregator, ..
            } => match sampler_aggregator {
                None | Some(SamplerAggregatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(SamplerAggregatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(SamplerAggregatorSourceSpec::Config { .. }) => None,
            },
            Self::PdfAdaptationImage {
                sampler_aggregator, ..
            } => match sampler_aggregator {
                None | Some(SamplerAggregatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(SamplerAggregatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(SamplerAggregatorSourceSpec::Config { .. }) => None,
            },
            Self::PdfAdaptationPlotLine {
                sampler_aggregator, ..
            } => match sampler_aggregator {
                None | Some(SamplerAggregatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(SamplerAggregatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(SamplerAggregatorSourceSpec::Config { .. }) => None,
            },
            Self::Image { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn sample_sampler_config(&self) -> Option<SamplerAggregatorConfig> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample {
                sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config { config }),
                ..
            } => Some(config.clone()),
            Self::Sample { .. } => None,
            Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn evaluator_source(&self) -> Option<SourceRefSpec> {
        match self {
            Self::SetAccumulator { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
            Self::Sample { evaluator, .. }
            | Self::Image { evaluator, .. }
            | Self::PlotLine { evaluator, .. } => match evaluator {
                None | Some(EvaluatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(EvaluatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(EvaluatorSourceSpec::Config { .. }) => None,
            },
        }
    }

    pub fn evaluator_config(&self) -> Option<EvaluatorConfig> {
        match self {
            Self::Sample {
                evaluator: Some(EvaluatorSourceSpec::Config { config }),
                ..
            }
            | Self::Image {
                evaluator: Some(EvaluatorSourceSpec::Config { config }),
                ..
            }
            | Self::PlotLine {
                evaluator: Some(EvaluatorSourceSpec::Config { config }),
                ..
            } => Some(config.clone()),
            _ => None,
        }
    }

    pub fn batch_transforms_config(&self) -> Option<Vec<BatchTransformConfig>> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample {
                batch_transforms, ..
            } => batch_transforms.clone(),
            Self::Image {
                batch_transforms, ..
            }
            | Self::PdfAdaptationImage {
                batch_transforms, ..
            }
            | Self::PdfAdaptationPlotLine {
                batch_transforms, ..
            }
            | Self::PlotLine {
                batch_transforms, ..
            } => batch_transforms.clone(),
            Self::ParameterScan { .. } | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn sample_accumulator_source(&self) -> Option<SourceRefSpec> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample { accumulator, .. } => match accumulator {
                None | Some(AccumulatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(AccumulatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(AccumulatorSourceSpec::Config { .. }) => None,
            },
            Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn source_task_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(SourceRefSpec::FromName(name)) = self.sample_sampler_source() {
            out.push(name);
        }
        if let Some(SourceRefSpec::FromName(name)) = self.evaluator_source() {
            out.push(name);
        }
        if let Some(SourceRefSpec::FromName(name)) = self.sample_accumulator_source() {
            out.push(name);
        }
        out
    }

    pub fn is_sourceable(&self) -> bool {
        true
    }

    pub fn new_accumulator_config(&self) -> Result<Option<AccumulatorConfig>, BuildError> {
        match self {
            Self::SetAccumulator { accumulator } => Ok(Some(accumulator.clone())),
            Self::Sample {
                accumulator: Some(AccumulatorSourceSpec::Config { config }),
                ..
            } => Ok(Some(config.clone())),
            Self::Sample { .. } => Ok(None),
            Self::PdfAdaptationImage { .. } | Self::PdfAdaptationPlotLine { .. } => {
                Ok(Some(AccumulatorConfig::Empty))
            }
            Self::Image { accumulator, .. } | Self::PlotLine { accumulator, .. } => {
                Ok(Some(accumulator.full_config()))
            }
            Self::ParameterScan { .. } | Self::HyperparameterTuning { .. } => Ok(None),
        }
    }

    pub fn nr_expected_samples(&self) -> Option<i64> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample { stop_condition, .. } => stop_condition.max_samples,
            Self::Image { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::PdfAdaptationImage { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::PdfAdaptationPlotLine { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::PlotLine { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::ParameterScan {
                parameter,
                parameters,
                ..
            } => parameter_scan_grid_len(parameter, parameters).map(|value| value as i64),
            Self::HyperparameterTuning {
                optimizer,
                parameters,
                ..
            } => hyperparameter_tuning_trial_count(optimizer, parameters).map(|count| count as i64),
        }
    }

    pub fn sample_stop_condition(&self) -> Option<&SampleStopCondition> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample { stop_condition, .. } => Some(stop_condition),
            Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn sample_measurement(&self) -> Option<&TaskMeasurementSpec> {
        match self {
            Self::Sample { measurement, .. } => measurement.as_ref(),
            Self::SetAccumulator { .. }
            | Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn effective_sample_measurement(&self) -> Option<TaskMeasurementSpec> {
        match self {
            Self::Sample { measurement, .. } => Some(measurement.clone().unwrap_or_default()),
            Self::SetAccumulator { .. }
            | Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn sample_queue_tuning(&self) -> Option<&SamplerQueueTuning> {
        match self {
            Self::SetAccumulator { .. } => None,
            Self::Sample { queue_tuning, .. } => queue_tuning.as_ref(),
            Self::Image { .. }
            | Self::PdfAdaptationImage { .. }
            | Self::PdfAdaptationPlotLine { .. }
            | Self::PlotLine { .. }
            | Self::ParameterScan { .. }
            | Self::HyperparameterTuning { .. } => None,
        }
    }

    pub fn set_sample_queue_tuning(
        &mut self,
        queue_tuning: Option<SamplerQueueTuning>,
    ) -> Result<(), String> {
        match self {
            Self::Sample {
                queue_tuning: current,
                ..
            } => {
                if let Some(next) = queue_tuning.as_ref() {
                    next.validate()?;
                }
                *current = queue_tuning;
                Ok(())
            }
            _ => Err("queue_tuning is only supported for sample tasks".to_string()),
        }
    }
}

fn hyperparameter_tuning_trial_count(
    optimizer: &HyperparameterTuningOptimizerSpec,
    parameters: &BTreeMap<String, HyperparameterTuningParameterDomain>,
) -> Option<usize> {
    match optimizer.algorithm {
        HyperparameterTuningAlgorithm::RandomSearch => optimizer
            .random_search_params()
            .ok()
            .map(|params| params.max_trials),
        HyperparameterTuningAlgorithm::GridSearch => {
            parameters.values().try_fold(1usize, |count, domain| {
                count.checked_mul(hyperparameter_grid_domain_len(domain)?)
            })
        }
    }
}

fn hyperparameter_grid_domain_len(domain: &HyperparameterTuningParameterDomain) -> Option<usize> {
    match domain {
        HyperparameterTuningParameterDomain::Float(_) => None,
        HyperparameterTuningParameterDomain::Categorical(domain) => Some(domain.values.len()),
        HyperparameterTuningParameterDomain::Integer(domain) => {
            let step = domain.step.unwrap_or(1);
            let span = domain.max.checked_sub(domain.min)?;
            Some((span / step + 1) as usize)
        }
    }
}

pub fn generated_task_name(task: &RunTaskSpec, sequence_nr: i32) -> String {
    format!("{}-{sequence_nr}", task.kind_str())
}

pub trait IntoPreflightTask: Sized {
    fn into_preflight(self) -> Result<Option<Self>, BuildError>;
}

impl IntoPreflightTask for RunTaskSpec {
    fn into_preflight(self) -> Result<Option<Self>, BuildError> {
        self.validate().map_err(BuildError::invalid_input)?;
        match self {
            Self::SetAccumulator { accumulator } => Ok(Some(Self::SetAccumulator { accumulator })),
            Self::Sample {
                mut stop_condition,
                measurement,
                evaluator,
                sampler_aggregator,
                accumulator,
                queue_tuning,
                batch_transforms,
            } => Ok(Some(Self::Sample {
                stop_condition: SampleStopCondition {
                    min_samples: stop_condition.min_samples.take(),
                    max_samples: Some(if stop_condition.max_samples == Some(0) {
                        0
                    } else {
                        1
                    }),
                    absolute_error: stop_condition.absolute_error.take(),
                    relative_error: stop_condition.relative_error.take(),
                    projection: stop_condition.projection.take(),
                    metric: stop_condition.metric.take(),
                },
                measurement,
                evaluator,
                sampler_aggregator,
                accumulator,
                queue_tuning,
                batch_transforms,
            })),
            Self::Image {
                mut geometry,
                accumulator,
                evaluator,
                display,
                batch_transforms,
            } => {
                geometry.reduce_for_preflight(4, 4);
                Ok(Some(Self::Image {
                    geometry,
                    accumulator,
                    evaluator,
                    display,
                    batch_transforms,
                }))
            }
            Self::PdfAdaptationImage {
                mut geometry,
                sampler_aggregator,
                batch_transforms,
            } => {
                geometry.reduce_for_preflight(4, 4);
                Ok(Some(Self::PdfAdaptationImage {
                    geometry,
                    sampler_aggregator,
                    batch_transforms,
                }))
            }
            Self::PdfAdaptationPlotLine {
                mut geometry,
                sampler_aggregator,
                batch_transforms,
            } => {
                geometry.reduce_for_preflight(8);
                Ok(Some(Self::PdfAdaptationPlotLine {
                    geometry,
                    sampler_aggregator,
                    batch_transforms,
                }))
            }
            Self::PlotLine {
                mut geometry,
                accumulator,
                evaluator,
                display,
                batch_transforms,
            } => {
                geometry.reduce_for_preflight(8);
                Ok(Some(Self::PlotLine {
                    geometry,
                    accumulator,
                    evaluator,
                    display,
                    batch_transforms,
                }))
            }
            Self::ParameterScan {
                parameter,
                parameters,
                measurement,
                trial_run_toml,
                max_concurrent_runs,
            } => Ok(Some(Self::ParameterScan {
                parameter,
                parameters,
                measurement,
                trial_run_toml,
                max_concurrent_runs,
            })),
            Self::HyperparameterTuning {
                optimizer,
                objective,
                parameters,
                trial_run_toml,
                max_concurrent_trials,
            } => Ok(Some(Self::HyperparameterTuning {
                optimizer,
                objective,
                parameters,
                trial_run_toml,
                max_concurrent_trials,
            })),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageDisplayMode {
    #[default]
    Auto,
    ScalarHeatmap,
    VectorMagnitude,
    ComplexPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineDisplayMode {
    #[default]
    Auto,
    ScalarCurve,
    Components,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotAccumulatorKind {
    Scalar,
    Vector,
}

impl PlotAccumulatorKind {
    pub fn components(self) -> Vec<String> {
        match self {
            Self::Scalar => vec!["value".to_string()],
            Self::Vector => vec!["real".to_string(), "imag".to_string()],
        }
    }

    pub fn full_config(self) -> AccumulatorConfig {
        AccumulatorConfig::FullVector {
            components: self.components(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Linspace {
    pub start: f64,
    pub stop: f64,
    pub count: usize,
}

impl Linspace {
    pub fn validate(&self, label: &str) -> Result<(), String> {
        if self.count == 0 {
            return Err(format!("{label} count must be > 0"));
        }
        if !self.start.is_finite() || !self.stop.is_finite() {
            return Err(format!("{label} bounds must be finite"));
        }
        Ok(())
    }

    pub fn reduce_for_preflight(&mut self, count: usize) {
        self.count = self.count.min(count).max(1);
    }

    pub fn value_at(&self, index: usize) -> f64 {
        if self.count <= 1 {
            return self.start;
        }
        let clamped_index = index.min(self.count - 1);
        let t = clamped_index as f64 / (self.count - 1) as f64;
        self.start + t * (self.stop - self.start)
    }

    /// Returns the `count` inclusive parameter values used by raster geometry.
    pub fn values(&self) -> impl Iterator<Item = f64> + '_ {
        (0..self.count).map(|index| self.value_at(index))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaneRasterGeometry {
    pub offset: Vec<f64>,
    pub u_vector: Vec<f64>,
    pub v_vector: Vec<f64>,
    pub u_linspace: Linspace,
    pub v_linspace: Linspace,
    #[serde(default)]
    pub discrete: Vec<i64>,
}

impl PlaneRasterGeometry {
    pub fn validate(&self) -> Result<(), String> {
        self.u_linspace.validate("u_linspace")?;
        self.v_linspace.validate("v_linspace")?;
        let dims = self.offset.len();
        if dims == 0 {
            return Err(
                "plane geometry offset must have at least one continuous dimension".to_string(),
            );
        }
        if self.u_vector.len() != dims || self.v_vector.len() != dims {
            return Err("plane geometry vectors must match offset dimensionality".to_string());
        }
        if vector_norm_sq(&self.u_vector) == 0.0 || vector_norm_sq(&self.v_vector) == 0.0 {
            return Err("plane geometry vectors must be non-zero".to_string());
        }
        if !vectors_are_independent(&self.u_vector, &self.v_vector) {
            return Err("plane geometry vectors must not be parallel".to_string());
        }
        Ok(())
    }

    pub fn nr_points(&self) -> usize {
        self.u_linspace.count.saturating_mul(self.v_linspace.count)
    }

    /// Maps a raster pixel index to `offset + t * u_vector + s * v_vector`.
    pub fn point_at(&self, index: usize) -> Vec<f64> {
        let width = self.u_linspace.count.max(1);
        let u_idx = index % width;
        let v_idx = index / width;
        self.point_at_indices(u_idx, v_idx)
    }

    pub fn point_at_indices(&self, u_idx: usize, v_idx: usize) -> Vec<f64> {
        let u = self.u_linspace.value_at(u_idx);
        let v = self.v_linspace.value_at(v_idx);
        self.offset
            .iter()
            .zip(self.u_vector.iter())
            .zip(self.v_vector.iter())
            .map(|((offset, basis_u), basis_v)| offset + u * basis_u + v * basis_v)
            .collect()
    }

    pub fn reduce_for_preflight(&mut self, u_count: usize, v_count: usize) {
        self.u_linspace.reduce_for_preflight(u_count);
        self.v_linspace.reduce_for_preflight(v_count);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LineRasterGeometry {
    pub offset: Vec<f64>,
    pub direction: Vec<f64>,
    pub linspace: Linspace,
    #[serde(default)]
    pub discrete: Vec<i64>,
}

impl LineRasterGeometry {
    pub fn validate(&self) -> Result<(), String> {
        self.linspace.validate("linspace")?;
        let dims = self.offset.len();
        if dims == 0 {
            return Err(
                "line geometry offset must have at least one continuous dimension".to_string(),
            );
        }
        if self.direction.len() != dims {
            return Err("line geometry direction must match offset dimensionality".to_string());
        }
        if vector_norm_sq(&self.direction) == 0.0 {
            return Err("line geometry direction must be non-zero".to_string());
        }
        Ok(())
    }

    pub fn nr_points(&self) -> usize {
        self.linspace.count
    }

    pub fn parameter_at(&self, index: usize) -> f64 {
        self.linspace.value_at(index)
    }

    /// Maps a raster line pixel index to `offset + t * direction`.
    pub fn point_at(&self, index: usize) -> Vec<f64> {
        let t = self.parameter_at(index);
        self.offset
            .iter()
            .zip(self.direction.iter())
            .map(|(offset, direction)| offset + t * direction)
            .collect()
    }

    pub fn reduce_for_preflight(&mut self, count: usize) {
        self.linspace.reduce_for_preflight(count);
    }
}

fn vector_norm_sq(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>()
}

fn vectors_are_independent(left: &[f64], right: &[f64]) -> bool {
    let left_norm = vector_norm_sq(left).sqrt();
    let right_norm = vector_norm_sq(right).sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        return false;
    }
    let cosine = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| l * r)
        .sum::<f64>()
        / (left_norm * right_norm);
    (1.0 - cosine.abs()) > 1e-9
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTask {
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string")]
    pub id: i64,
    pub run_id: i32,
    pub name: String,
    pub sequence_nr: i32,
    pub task: RunTaskSpec,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    pub spawned_from_snapshot_id: Option<i64>,
    pub state: RunTaskState,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub failure_reason: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub task_toml: String,
    pub measurement_output: Option<TaskMeasurementOutput>,
    pub controller_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskMeasurementOutput {
    Completed { results: Vec<MeasurementResult> },
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub task: RunTaskSpec,
}

impl RunTaskInput {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = self.name.as_deref() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err("task name must be non-empty when set".to_string());
            }
            if trimmed != name {
                return Err("task name cannot have leading or trailing whitespace".to_string());
            }
        }
        self.task.validate()
    }
}

#[derive(Serialize)]
struct TaskTomlFile<'a> {
    task: &'a RunTaskInput,
}

pub fn canonical_task_toml(task: &RunTaskInput) -> Result<String, toml::ser::Error> {
    toml::to_string(&TaskTomlFile { task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TrainingProjection;
    use crate::sampling::{HavanaSamplerParams, NaiveMonteCarloSamplerParams};

    #[test]
    fn sample_task_without_accumulator_reuses_previous_state() {
        let task = RunTaskSpec::Sample {
            stop_condition: SampleStopCondition {
                max_samples: Some(10),
                ..SampleStopCondition::default()
            },
            measurement: None,
            evaluator: None,
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::NaiveMonteCarlo {
                    params: NaiveMonteCarloSamplerParams::default(),
                },
            }),
            accumulator: Some(AccumulatorSourceSpec::Latest("latest".to_string())),
            queue_tuning: None,
            batch_transforms: Some(Vec::new()),
        };

        assert_eq!(task.new_accumulator_config().unwrap(), None);
    }

    #[test]
    fn sample_stop_condition_accepts_metric_target() {
        let stop_condition: SampleStopCondition = toml::from_str(
            r#"
min_samples = 100
relative_error = 0.1
metric = { name = "variance", component = "real" }
"#,
        )
        .expect("stop condition");

        stop_condition.validate().expect("valid stop condition");
        assert_eq!(stop_condition.min_samples, Some(100));
        assert_eq!(
            stop_condition.metric.as_ref().map(|metric| metric.name),
            Some(AccumulatorMetricName::Variance)
        );
    }

    #[test]
    fn sample_stop_condition_accepts_time_normalized_variance_metric_target() {
        let stop_condition: SampleStopCondition = toml::from_str(
            r#"
min_samples = 100
relative_error = 0.1
metric = { name = "time_normalized_variance", component = "real" }
"#,
        )
        .expect("stop condition");

        stop_condition.validate().expect("valid stop condition");
        assert_eq!(
            stop_condition.metric.as_ref().map(|metric| metric.name),
            Some(AccumulatorMetricName::TimeNormalizedVariance)
        );
    }

    #[test]
    fn measurement_spec_defaults_to_central_value() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            measurement: MeasurementSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[measurement]
source_task = "sample"
"#,
        )
        .expect("measurement spec");

        wrapper.measurement.validate().expect("valid measurement");
        let selector = wrapper.measurement.metric_selector();
        assert_eq!(selector.name, AccumulatorMetricName::Mean);
        assert_eq!(wrapper.measurement.explicit_metric_selector(), None);
        assert_eq!(
            wrapper.measurement.quantity,
            MeasurementQuantitySpec::Name(MeasurementQuantityName::CentralValue)
        );
    }

    #[test]
    fn sample_task_parses_task_local_measurement() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            task: RunTaskSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[task]
kind = "sample"

[task.stop_condition]
max_samples = 100000
metric = { name = "time_normalized_variance" }

[task.measurement]
quantity = { metric = "time_normalized_variance" }
mode = "minimize"
"#,
        )
        .expect("sample task");

        wrapper.task.validate().expect("valid sample task");
        let measurement = wrapper
            .task
            .sample_measurement()
            .expect("task-local measurement");
        assert_eq!(
            measurement.metric_selector().name,
            AccumulatorMetricName::TimeNormalizedVariance
        );
        assert_eq!(measurement.mode, MeasurementMode::Minimize);
    }

    #[test]
    fn measurement_spec_parses_metric_alias() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            measurement: MeasurementSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[measurement]
source_task = "sample"
metric = "time_normalized_variance"
mode = "minimize"
"#,
        )
        .expect("measurement spec");

        wrapper.measurement.validate().expect("valid measurement");
        let selector = wrapper.measurement.metric_selector();
        assert_eq!(selector.name, AccumulatorMetricName::TimeNormalizedVariance);
        assert_eq!(wrapper.measurement.mode, MeasurementMode::Minimize);
    }

    #[test]
    fn measurement_spec_parses_component_qualified_metric() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            measurement: MeasurementSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[measurement]
source_task = "sample"
metric = { component = "real", name = "variance" }
"#,
        )
        .expect("measurement spec");

        wrapper.measurement.validate().expect("valid measurement");
        let selector = wrapper
            .measurement
            .explicit_metric_selector()
            .expect("metric selector");
        assert_eq!(selector.name, AccumulatorMetricName::Variance);
        assert_eq!(selector.component.as_deref(), Some("real"));
    }

    #[test]
    fn measurement_spec_parses_quantity_metric() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            measurement: MeasurementSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[measurement]
source_task = "sample"
quantity = { component = "real", metric = "time_normalized_variance" }
"#,
        )
        .expect("measurement spec");

        wrapper.measurement.validate().expect("valid measurement");
        let selector = wrapper
            .measurement
            .explicit_metric_selector()
            .expect("metric selector");
        assert_eq!(selector.name, AccumulatorMetricName::TimeNormalizedVariance);
        assert_eq!(selector.component.as_deref(), Some("real"));
    }

    #[test]
    fn hyperparameter_tuning_task_parses_random_search_config_shape() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            task: RunTaskSpec,
        }

        let wrapper: Wrapper = toml::from_str(
            r#"
[task]
kind = "hyperparameter_tuning"
max_concurrent_trials = 2
trial_run_toml = "name = \"trial\"\n"

[task.optimizer]
algorithm = "random_search"
seed = 1

[task.optimizer.params]
max_trials = 8

[task.objective]
source_task = "sample"
mode = "minimize"
quantity = { metric = "time_normalized_variance" }

[task.parameters.mu_scale]
kind = "float"
min = 0.0
max = 1.0

[task.parameters.bins]
kind = "integer"
min = 16
max = 128
step = 8

[task.parameters.mode]
kind = "categorical"
values = ["auto", "none"]
"#,
        )
        .expect("hyperparameter tuning task shape");

        let RunTaskSpec::HyperparameterTuning {
            optimizer,
            objective,
            parameters,
            max_concurrent_trials,
            ..
        } = wrapper.task
        else {
            panic!("expected hyperparameter tuning task");
        };

        assert_eq!(optimizer.random_search_params().unwrap().max_trials, 8);
        assert_eq!(optimizer.seed, Some(1));
        assert_eq!(
            optimizer.algorithm,
            HyperparameterTuningAlgorithm::RandomSearch
        );
        assert_eq!(objective.source_task, "sample");
        assert_eq!(parameters.len(), 3);
        assert_eq!(max_concurrent_trials, 2);
        optimizer.validate().expect("valid optimizer");
    }

    #[test]
    fn hyperparameter_tuning_parameter_domains_validate() {
        let float = HyperparameterTuningParameterDomain::Float(HyperparameterTuningFloatDomain {
            min: 0.0,
            max: 1.0,
        });
        float.validate("mu").expect("valid float");

        let int = HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
            min: 1,
            max: 4,
            step: Some(1),
        });
        int.validate("bins").expect("valid integer");

        let categorical = HyperparameterTuningParameterDomain::Categorical(
            HyperparameterTuningCategoricalDomain {
                values: vec![toml::Value::String("auto".to_string())],
            },
        );
        categorical.validate("mode").expect("valid categorical");

        let bad_step =
            HyperparameterTuningParameterDomain::Integer(HyperparameterTuningIntegerDomain {
                min: 1,
                max: 4,
                step: Some(0),
            });
        assert!(bad_step.validate("bins").is_err());
    }

    #[test]
    fn set_accumulator_task_requests_fresh_accumulator_state() {
        let accumulator = AccumulatorConfig::vector(
            vec!["real".to_string(), "imag".to_string()],
            TrainingProjection::Norm,
        );
        let task = RunTaskSpec::SetAccumulator {
            accumulator: accumulator.clone(),
        };

        assert_eq!(task.kind_str(), "set_accumulator");
        assert_eq!(task.new_accumulator_config().unwrap(), Some(accumulator));
        assert_eq!(task.sample_stop_condition(), None);
    }

    #[test]
    fn canonical_task_toml_wraps_single_task_payload() {
        let input = RunTaskInput {
            name: Some("sample-1".to_string()),
            task: RunTaskSpec::Sample {
                stop_condition: SampleStopCondition {
                    max_samples: Some(10),
                    ..SampleStopCondition::default()
                },
                measurement: None,
                evaluator: None,
                sampler_aggregator: None,
                accumulator: None,
                queue_tuning: None,
                batch_transforms: None,
            },
        };

        let toml = canonical_task_toml(&input).expect("task toml");

        assert!(toml.contains("[task]"));
        assert!(toml.contains("name = \"sample-1\""));
        assert!(toml.contains("kind = \"sample\""));
    }

    #[test]
    fn sample_task_rejects_dual_source() {
        let missing = RunTaskSpec::Sample {
            stop_condition: SampleStopCondition {
                max_samples: Some(0),
                ..SampleStopCondition::default()
            },
            measurement: None,
            evaluator: None,
            sampler_aggregator: None,
            accumulator: None,
            queue_tuning: None,
            batch_transforms: None,
        };
        assert!(missing.validate().is_ok());

        let both = RunTaskSpec::Sample {
            stop_condition: SampleStopCondition {
                max_samples: Some(0),
                ..SampleStopCondition::default()
            },
            measurement: None,
            evaluator: None,
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Latest("latest".to_string())),
            accumulator: None,
            queue_tuning: None,
            batch_transforms: None,
        };
        assert!(both.validate().is_ok());
    }

    #[test]
    fn sample_task_with_havana_training_requires_budget_in_config_mode() {
        let task = RunTaskSpec::Sample {
            stop_condition: SampleStopCondition::default(),
            measurement: None,
            evaluator: None,
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::HavanaTraining {
                    params: HavanaSamplerParams::default(),
                },
            }),
            accumulator: None,
            queue_tuning: None,
            batch_transforms: None,
        };
        assert!(task.validate().is_err());
    }

    #[test]
    fn linspace_includes_bounds_and_interpolates_by_count() {
        let linspace = Linspace {
            start: 0.0,
            stop: 1.0,
            count: 5,
        };

        assert_eq!(
            linspace.values().collect::<Vec<_>>(),
            vec![0.0, 0.25, 0.5, 0.75, 1.0]
        );
        assert_eq!(linspace.value_at(100), 1.0);
    }

    #[test]
    fn raster_geometries_use_linspace_coordinates() {
        let plane = PlaneRasterGeometry {
            offset: vec![0.0, 0.0, 0.0],
            u_vector: vec![1.0, 0.0, 0.0],
            v_vector: vec![0.0, 1.0, 0.0],
            u_linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 3,
            },
            v_linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 3,
            },
            discrete: Vec::new(),
        };
        let line = LineRasterGeometry {
            offset: vec![0.0, 0.0, 0.0],
            direction: vec![1.0, 2.0, 3.0],
            linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 3,
            },
            discrete: Vec::new(),
        };

        assert_eq!(plane.point_at(0), vec![0.0, 0.0, 0.0]);
        assert_eq!(plane.point_at(4), vec![0.5, 0.5, 0.0]);
        assert_eq!(plane.point_at(8), vec![1.0, 1.0, 0.0]);
        assert_eq!(line.point_at(0), vec![0.0, 0.0, 0.0]);
        assert_eq!(line.point_at(1), vec![0.5, 1.0, 1.5]);
        assert_eq!(line.point_at(2), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn plotting_tasks_always_request_fresh_full_observables() {
        let image = RunTaskSpec::Image {
            geometry: PlaneRasterGeometry {
                offset: vec![0.0, 0.0],
                u_vector: vec![1.0, 0.0],
                v_vector: vec![0.0, 1.0],
                u_linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                v_linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            accumulator: PlotAccumulatorKind::Vector,
            evaluator: None,
            display: ImageDisplayMode::Auto,
            batch_transforms: None,
        };
        let line = RunTaskSpec::PlotLine {
            geometry: LineRasterGeometry {
                offset: vec![0.0, 0.0],
                direction: vec![1.0, 0.0],
                linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            accumulator: PlotAccumulatorKind::Scalar,
            evaluator: None,
            display: LineDisplayMode::Auto,
            batch_transforms: None,
        };

        assert_eq!(
            image.new_accumulator_config().unwrap(),
            Some(AccumulatorConfig::FullVector {
                components: vec!["real".to_string(), "imag".to_string()]
            })
        );
        assert_eq!(
            line.new_accumulator_config().unwrap(),
            Some(AccumulatorConfig::FullVector {
                components: vec!["value".to_string()]
            })
        );
    }

    #[test]
    fn pdf_adaptation_task_defaults_sampler_source_to_latest() {
        let task = RunTaskSpec::PdfAdaptationImage {
            geometry: PlaneRasterGeometry {
                offset: vec![0.0, 0.0],
                u_vector: vec![1.0, 0.0],
                v_vector: vec![0.0, 1.0],
                u_linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 4,
                },
                v_linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 4,
                },
                discrete: Vec::new(),
            },
            sampler_aggregator: None,
            batch_transforms: None,
        };

        assert_eq!(task.sample_sampler_source(), Some(SourceRefSpec::Latest));
        assert_eq!(
            task.new_accumulator_config().unwrap(),
            Some(AccumulatorConfig::Empty)
        );
    }

    #[test]
    fn pdf_adaptation_task_rejects_inline_sampler_config() {
        let task = RunTaskSpec::PdfAdaptationImage {
            geometry: PlaneRasterGeometry {
                offset: vec![0.0, 0.0],
                u_vector: vec![1.0, 0.0],
                v_vector: vec![0.0, 1.0],
                u_linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 4,
                },
                v_linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 4,
                },
                discrete: Vec::new(),
            },
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::NaiveMonteCarlo {
                    params: NaiveMonteCarloSamplerParams::default(),
                },
            }),
            batch_transforms: None,
        };

        assert!(task.validate().is_err());
    }

    #[test]
    fn pdf_adaptation_plot_line_task_defaults_sampler_source_to_latest() {
        let task = RunTaskSpec::PdfAdaptationPlotLine {
            geometry: LineRasterGeometry {
                offset: vec![0.0, 0.0],
                direction: vec![1.0, 0.0],
                linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            sampler_aggregator: None,
            batch_transforms: None,
        };

        assert_eq!(task.sample_sampler_source(), Some(SourceRefSpec::Latest));
        assert_eq!(
            task.new_accumulator_config().unwrap(),
            Some(AccumulatorConfig::Empty)
        );
    }

    #[test]
    fn pdf_adaptation_plot_line_task_rejects_inline_sampler_config() {
        let task = RunTaskSpec::PdfAdaptationPlotLine {
            geometry: LineRasterGeometry {
                offset: vec![0.0, 0.0],
                direction: vec![1.0, 0.0],
                linspace: Linspace {
                    start: 0.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::NaiveMonteCarlo {
                    params: NaiveMonteCarloSamplerParams::default(),
                },
            }),
            batch_transforms: None,
        };

        assert!(task.validate().is_err());
    }
}
