use crate::core::{BuildError, EngineError};
use crate::core::{LineRasterGeometry, PlaneRasterGeometry};
use crate::evaluation::{Batch, PointSpec};
use crate::sampling::{LatentBatchSpec, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use ndarray::Array2;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RasterPlaneSamplerParams {
    pub geometry: PlaneRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RasterLineSamplerParams {
    pub geometry: LineRasterGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPlaneSamplerSnapshot {
    params: RasterPlaneSamplerParams,
    next_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterLineSamplerSnapshot {
    params: RasterLineSamplerParams,
    next_index: usize,
}

pub struct RasterPlaneSampler {
    params: RasterPlaneSamplerParams,
    next_index: usize,
}

pub struct RasterLineSampler {
    params: RasterLineSamplerParams,
    next_index: usize,
}

impl RasterPlaneSampler {
    pub fn from_params_and_point_spec(
        params: RasterPlaneSamplerParams,
        point_spec: &PointSpec,
    ) -> Result<Self, BuildError> {
        validate_plane_geometry(&params.geometry, point_spec)?;
        Ok(Self {
            params,
            next_index: 0,
        })
    }

    pub fn from_snapshot(
        snapshot: RasterPlaneSamplerSnapshot,
        point_spec: &PointSpec,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_point_spec(snapshot.params, point_spec)?;
        Ok(Self {
            next_index: snapshot.next_index,
            ..sampler
        })
    }

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        let width = self.params.geometry.u_linspace.count;
        let u_idx = index % width;
        let v_idx = index / width;
        let u = linspace_value(&self.params.geometry.u_linspace, u_idx);
        let v = linspace_value(&self.params.geometry.v_linspace, v_idx);
        self.params
            .geometry
            .offset
            .iter()
            .zip(self.params.geometry.u_vector.iter())
            .zip(self.params.geometry.v_vector.iter())
            .map(|((offset, basis_u), basis_v)| offset + u * basis_u + v * basis_v)
            .collect()
    }
}

impl RasterLineSampler {
    pub fn from_params_and_point_spec(
        params: RasterLineSamplerParams,
        point_spec: &PointSpec,
    ) -> Result<Self, BuildError> {
        validate_line_geometry(&params.geometry, point_spec)?;
        Ok(Self {
            params,
            next_index: 0,
        })
    }

    pub fn from_snapshot(
        snapshot: RasterLineSamplerSnapshot,
        point_spec: &PointSpec,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_point_spec(snapshot.params, point_spec)?;
        Ok(Self {
            next_index: snapshot.next_index,
            ..sampler
        })
    }

    fn total_samples(&self) -> usize {
        self.params.geometry.nr_points()
    }

    fn point_at(&self, index: usize) -> Vec<f64> {
        let t = linspace_value(&self.params.geometry.linspace, index);
        self.params
            .geometry
            .offset
            .iter()
            .zip(self.params.geometry.direction.iter())
            .map(|(offset, direction)| offset + t * direction)
            .collect()
    }
}

impl SamplerAggregator for RasterPlaneSampler {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        validate_plane_geometry(&self.params.geometry, point_spec)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "raster plane sampler cannot produce an empty batch",
            ));
        }
        let dims = self.params.geometry.offset.len();
        let mut continuous = Array2::<f64>::zeros((nr_samples, dims));
        for row_idx in 0..nr_samples {
            let point = self.point_at(self.next_index + row_idx);
            for (col_idx, value) in point.into_iter().enumerate() {
                continuous[(row_idx, col_idx)] = value;
            }
        }
        let (continuous_data, offset) = continuous.into_raw_vec_and_offset();
        debug_assert_eq!(offset, Some(0));
        let batch = Batch::from_flat_data(
            nr_samples,
            dims,
            self.params.geometry.discrete.len(),
            continuous_data,
            self.params.geometry.discrete.repeat(nr_samples),
        )
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_weights(&mut self, _training_weights: &[f64]) -> Result<(), EngineError> {
        Ok(())
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::RasterPlane {
            raw: serde_json::to_value(RasterPlaneSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

impl SamplerAggregator for RasterLineSampler {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        validate_line_geometry(&self.params.geometry, point_spec)
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        if remaining == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce {
                nr_samples: remaining,
            })
        }
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let remaining = self.total_samples().saturating_sub(self.next_index);
        let nr_samples = nr_samples.min(remaining);
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "raster line sampler cannot produce an empty batch",
            ));
        }
        let dims = self.params.geometry.offset.len();
        let mut continuous = Array2::<f64>::zeros((nr_samples, dims));
        for row_idx in 0..nr_samples {
            let point = self.point_at(self.next_index + row_idx);
            for (col_idx, value) in point.into_iter().enumerate() {
                continuous[(row_idx, col_idx)] = value;
            }
        }
        let (continuous_data, offset) = continuous.into_raw_vec_and_offset();
        debug_assert_eq!(offset, Some(0));
        let batch = Batch::from_flat_data(
            nr_samples,
            dims,
            self.params.geometry.discrete.len(),
            continuous_data,
            self.params.geometry.discrete.repeat(nr_samples),
        )
        .map_err(|err| EngineError::engine(err.to_string()))?;
        self.next_index += nr_samples;
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_weights(&mut self, _training_weights: &[f64]) -> Result<(), EngineError> {
        Ok(())
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::RasterLine {
            raw: serde_json::to_value(RasterLineSamplerSnapshot {
                params: self.params.clone(),
                next_index: self.next_index,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

fn validate_plane_geometry(
    geometry: &PlaneRasterGeometry,
    point_spec: &PointSpec,
) -> Result<(), BuildError> {
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != point_spec.continuous_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry continuous dimension mismatch: expected {}, got {}",
            point_spec.continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != point_spec.discrete_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry discrete dimension mismatch: expected {}, got {}",
            point_spec.discrete_dims,
            geometry.discrete.len()
        )));
    }
    Ok(())
}

fn validate_line_geometry(
    geometry: &LineRasterGeometry,
    point_spec: &PointSpec,
) -> Result<(), BuildError> {
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != point_spec.continuous_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry continuous dimension mismatch: expected {}, got {}",
            point_spec.continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != point_spec.discrete_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry discrete dimension mismatch: expected {}, got {}",
            point_spec.discrete_dims,
            geometry.discrete.len()
        )));
    }
    Ok(())
}

fn linspace_value(linspace: &crate::core::Linspace, index: usize) -> f64 {
    if linspace.count <= 1 {
        return linspace.start;
    }
    let t = index as f64 / (linspace.count - 1) as f64;
    linspace.start + t * (linspace.stop - linspace.start)
}
