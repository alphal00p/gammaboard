use symbolica::numerical_integration::{ContinuousGrid, DiscreteGrid, Grid, Sample};

use crate::core::{BuildError, EngineError};
use crate::evaluation::Point;
use crate::sampling::HavanaSamplerParams;
use crate::utils::domain::Domain;

const DEFAULT_DISCRETE_MAX_PROB_RATIO: f64 = 30.0;

pub(crate) fn build_havana_grid(
    domain: &Domain,
    params: &HavanaSamplerParams,
) -> Result<Grid<f64>, BuildError> {
    match domain {
        Domain::Continuous { dims } => {
            if *dims == 0 {
                return Err(BuildError::build(
                    "havana sampler requires continuous_dims > 0",
                ));
            }
            Ok(Grid::Continuous(ContinuousGrid::new(
                *dims,
                params.bins,
                params.samples_for_update,
                None,
                false,
            )))
        }
        Domain::Discrete { branches, .. } => {
            if branches.is_empty() {
                return Err(BuildError::build(
                    "havana sampler requires at least one discrete branch",
                ));
            }
            let bins = branches
                .iter()
                .map(|branch| build_havana_grid(branch.domain.as_ref(), params).map(Some))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Grid::Discrete(DiscreteGrid::new(
                bins,
                DEFAULT_DISCRETE_MAX_PROB_RATIO,
                false,
            )))
        }
    }
}

pub(crate) fn validate_havana_grid_domain(
    grid: &Grid<f64>,
    domain: &Domain,
    context: &str,
) -> Result<(), BuildError> {
    match (grid, domain) {
        (Grid::Continuous(grid), Domain::Continuous { dims }) => {
            let actual = grid.continuous_dimensions.len();
            if actual != *dims {
                return Err(BuildError::build(format!(
                    "{context} expects continuous_dims={actual}, got {dims}",
                )));
            }
            Ok(())
        }
        (Grid::Discrete(grid), Domain::Discrete { branches, .. }) => {
            if grid.bins.len() != branches.len() {
                return Err(BuildError::build(format!(
                    "{context} expects {} discrete branches, got {}",
                    grid.bins.len(),
                    branches.len()
                )));
            }
            for (branch, bin) in branches.iter().zip(grid.bins.iter()) {
                let Some(sub_grid) = bin.sub_grid.as_ref() else {
                    return Err(BuildError::build(format!(
                        "{context} is missing a nested grid for discrete branch {}",
                        branch.index
                    )));
                };
                validate_havana_grid_domain(sub_grid, branch.domain.as_ref(), context)?;
            }
            Ok(())
        }
        (Grid::Uniform(_, _), _) => Err(BuildError::build(format!(
            "{context} does not support uniform grids"
        ))),
        (Grid::Continuous(_), Domain::Discrete { .. }) => Err(BuildError::build(format!(
            "{context} expects discrete dimensions, got a continuous grid"
        ))),
        (Grid::Discrete(_), Domain::Continuous { .. }) => Err(BuildError::build(format!(
            "{context} expects a continuous domain, got a discrete grid"
        ))),
    }
}

pub(crate) fn sample_to_point(sample: &Sample<f64>) -> Result<Point, EngineError> {
    fn discrete_index(index: usize) -> Result<i64, EngineError> {
        i64::try_from(index)
            .map_err(|_| EngineError::engine(format!("discrete index {index} does not fit in i64")))
    }

    fn recurse(
        sample: &Sample<f64>,
        discrete: &mut Vec<i64>,
    ) -> Result<(Vec<f64>, f64), EngineError> {
        match sample {
            Sample::Continuous(weight, continuous) => Ok((continuous.clone(), *weight)),
            Sample::Discrete(weight, index, maybe_child) => {
                discrete.push(discrete_index(*index)?);
                let Some(child) = maybe_child.as_ref() else {
                    return Err(EngineError::engine(
                        "havana sampler expected nested continuous samples",
                    ));
                };
                let (continuous, _) = recurse(child, discrete)?;
                Ok((continuous, *weight))
            }
            Sample::Uniform(weight, bin_indices, continuous) => {
                discrete.extend(
                    bin_indices
                        .iter()
                        .copied()
                        .map(discrete_index)
                        .collect::<Result<Vec<_>, _>>()?,
                );
                Ok((continuous.clone(), *weight))
            }
        }
    }

    let mut discrete = Vec::new();
    let (continuous, weight) = recurse(sample, &mut discrete)?;
    Ok(Point::new(continuous, discrete, weight))
}
