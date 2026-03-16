mod havana;
mod naive_monte_carlo;

use crate::engines::{BuildError, PointSpec, SamplerAggregator, SamplerAggregatorSnapshot};

pub use self::havana::HavanaSamplerParams;
use self::havana::{HavanaSampler, HavanaSamplerSnapshot};
use self::naive_monte_carlo::NaiveMonteCarloSamplerAggregator;
pub use self::naive_monte_carlo::NaiveMonteCarloSamplerParams;

impl SamplerAggregatorSnapshot {
    pub fn into_runtime(
        self,
        point_spec: &PointSpec,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { raw } => {
                let snapshot: NaiveMonteCarloSamplerAggregator = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode naive_monte_carlo sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(NaiveMonteCarloSamplerAggregator::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
            Self::Havana { raw } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
        }
    }
}

impl crate::engines::SamplerAggregatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::NaiveMonteCarlo { .. } => "naive_monte_carlo",
            Self::Havana { .. } => "havana",
        }
    }

    pub fn build(&self, point_spec: PointSpec) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { params } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_params_and_point_spec(
                    params.clone(),
                    &point_spec,
                )?,
            )),
            Self::Havana { params } => Ok(Box::new(HavanaSampler::from_params_and_point_spec(
                params.clone(),
                &point_spec,
            )?)),
        }
    }
}
