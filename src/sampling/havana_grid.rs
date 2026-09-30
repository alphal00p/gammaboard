use symbolica::numerical_integration::{ContinuousGrid, DiscreteGrid, Grid, Sample};

use crate::core::{BuildError, EngineError, EngineResultExt};
use crate::evaluation::Point;
use crate::sampling::HavanaSamplerParams;
use crate::utils::domain::Domain;

const DEFAULT_DISCRETE_MAX_PROB_RATIO: f64 = 30.0;

pub(crate) fn build_havana_grid(
    domain: &Domain,
    params: &HavanaSamplerParams,
) -> Result<Grid<f64>, BuildError> {
    crate::activate_symbolica_oem_license().map_err(|err| BuildError::build(err.to_string()))?;
    match domain {
        Domain::Continuous { dims } => {
            if *dims == 0 {
                return Err(BuildError::build(
                    "havana sampler requires continuous_dims > 0",
                ));
            }
            Ok(Grid::Continuous(
                ContinuousGrid::new(*dims, params.bins, params.samples_for_update, None, false)
                    .build_err()?,
            ))
        }
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => build_rectangular_havana_grid(*continuous_dims, discrete_cardinalities, params),
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
            Ok(Grid::Discrete(
                DiscreteGrid::new(bins, DEFAULT_DISCRETE_MAX_PROB_RATIO, false).build_err()?,
            ))
        }
    }
}

fn build_rectangular_havana_grid(
    continuous_dims: usize,
    discrete_cardinalities: &[usize],
    params: &HavanaSamplerParams,
) -> Result<Grid<f64>, BuildError> {
    if let Some((&cardinality, tail)) = discrete_cardinalities.split_first() {
        if cardinality == 0 {
            return Err(BuildError::build(
                "havana sampler requires rectangular discrete cardinalities > 0",
            ));
        }
        let bins = (0..cardinality)
            .map(|_| build_rectangular_havana_grid(continuous_dims, tail, params).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Grid::Discrete(
            DiscreteGrid::new(bins, DEFAULT_DISCRETE_MAX_PROB_RATIO, false).build_err()?,
        ));
    }
    build_havana_grid(&Domain::continuous(continuous_dims), params)
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
        (
            grid,
            Domain::Rectangular {
                discrete_cardinalities,
                continuous_dims,
            },
        ) => validate_rectangular_havana_grid(
            grid,
            *continuous_dims,
            discrete_cardinalities,
            context,
        ),
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

fn validate_rectangular_havana_grid(
    grid: &Grid<f64>,
    continuous_dims: usize,
    discrete_cardinalities: &[usize],
    context: &str,
) -> Result<(), BuildError> {
    if let Some((&cardinality, tail)) = discrete_cardinalities.split_first() {
        let Grid::Discrete(grid) = grid else {
            return Err(BuildError::build(format!(
                "{context} expects rectangular discrete dimensions, got a continuous grid"
            )));
        };
        if grid.bins.len() != cardinality {
            return Err(BuildError::build(format!(
                "{context} expects {cardinality} rectangular discrete branches, got {}",
                grid.bins.len()
            )));
        }
        for bin in &grid.bins {
            let Some(sub_grid) = bin.sub_grid.as_ref() else {
                return Err(BuildError::build(format!(
                    "{context} is missing a nested grid for a rectangular discrete branch",
                )));
            };
            validate_rectangular_havana_grid(sub_grid, continuous_dims, tail, context)?;
        }
        return Ok(());
    }
    validate_havana_grid_domain(grid, &Domain::continuous(continuous_dims), context)
}

/// Borrow coordinates while reusing storage for the discrete path.
pub(crate) fn sample_components<'a>(
    sample: &'a Sample<f64>,
    discrete: &mut Vec<i64>,
) -> Result<(&'a [f64], f64), EngineError> {
    fn discrete_index(index: usize) -> Result<i64, EngineError> {
        i64::try_from(index)
            .map_err(|_| EngineError::engine(format!("discrete index {index} does not fit in i64")))
    }

    let weight = sample.get_weight();
    let mut sample = sample;
    discrete.clear();
    loop {
        match sample {
            Sample::Continuous(_, continuous) => return Ok((continuous, weight)),
            Sample::Discrete(_, index, maybe_child) => {
                discrete.push(discrete_index(*index)?);
                let Some(child) = maybe_child.as_ref() else {
                    return Err(EngineError::engine(
                        "havana sampler expected nested continuous samples",
                    ));
                };
                sample = child;
            }
            Sample::Uniform(_, bin_indices, continuous) => {
                for &index in bin_indices {
                    discrete.push(discrete_index(index)?);
                }
                return Ok((continuous, weight));
            }
        }
    }
}

pub(crate) fn sample_to_point(sample: &Sample<f64>) -> Result<Point, EngineError> {
    let mut discrete = Vec::new();
    let (continuous, weight) = sample_components(sample, &mut discrete)?;
    Ok(Point::new(continuous.to_vec(), discrete, weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_components_preserve_outer_weight_and_clear_reused_paths() {
        let mut discrete = vec![99];
        for (sample, path, coordinates, weight) in [
            (
                Sample::Discrete(
                    3.0,
                    1,
                    Some(Box::new(Sample::Uniform(6.0, vec![2, 4], vec![0.1, 0.2]))),
                ),
                vec![1, 2, 4],
                vec![0.1, 0.2],
                3.0,
            ),
            (Sample::Continuous(2.0, vec![0.3]), vec![], vec![0.3], 2.0),
            (Sample::Uniform(4.0, vec![5], vec![]), vec![5], vec![], 4.0),
        ] {
            let (actual, actual_weight) = sample_components(&sample, &mut discrete).unwrap();
            assert_eq!(actual, coordinates);
            assert_eq!(actual_weight, weight);
            assert_eq!(discrete, path);
        }
        assert!(sample_components(&Sample::Discrete(1.0, 0, None), &mut discrete).is_err());
        assert!(
            sample_components(
                &Sample::Uniform(1.0, vec![usize::MAX], vec![]),
                &mut discrete
            )
            .is_err()
        );
    }
}
