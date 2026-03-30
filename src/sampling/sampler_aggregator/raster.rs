use crate::core::{BuildError, EngineError};
use crate::core::{LineRasterGeometry, PlaneRasterGeometry};
use crate::evaluation::{Batch, Point};
use crate::sampling::{LatentBatchSpec, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use crate::utils::domain::Domain;
use num::Integer;
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
    stride: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterLineSamplerSnapshot {
    params: RasterLineSamplerParams,
    next_index: usize,
    stride: usize,
}

pub struct RasterPlaneSampler {
    params: RasterPlaneSamplerParams,
    next_index: usize,
    stride: usize,
}

pub struct RasterLineSampler {
    params: RasterLineSamplerParams,
    next_index: usize,
    stride: usize,
}

impl RasterPlaneSampler {
    pub fn from_params_and_domain(
        params: RasterPlaneSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_plane_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
        })
    }

    pub fn from_snapshot(
        snapshot: RasterPlaneSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_domain(snapshot.params, domain)?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
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

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }
}

impl RasterLineSampler {
    pub fn from_params_and_domain(
        params: RasterLineSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_line_geometry(&params.geometry, domain)?;
        let total_samples = params.geometry.nr_points();
        Ok(Self {
            params,
            next_index: 0,
            stride: coprime_stride(total_samples),
        })
    }

    pub fn from_snapshot(
        snapshot: RasterLineSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let sampler = Self::from_params_and_domain(snapshot.params, domain)?;
        Ok(Self {
            next_index: snapshot.next_index,
            stride: snapshot.stride,
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

    fn permuted_index(&self, index: usize) -> usize {
        permuted_raster_index(index, self.total_samples(), self.stride)
    }
}

impl SamplerAggregator for RasterPlaneSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_plane_geometry(&self.params.geometry, domain)
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
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
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
                stride: self.stride,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

impl SamplerAggregator for RasterLineSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_line_geometry(&self.params.geometry, domain)
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
        let batch = Batch::from_points((0..nr_samples).map(|row_idx| {
            Point::new(
                self.point_at(self.permuted_index(self.next_index + row_idx)),
                self.params.geometry.discrete.clone(),
                1.0,
            )
        }))
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
                stride: self.stride,
            })
            .map_err(|err| EngineError::engine(err.to_string()))?,
        })
    }
}

fn validate_plane_geometry(
    geometry: &PlaneRasterGeometry,
    domain: &Domain,
) -> Result<(), BuildError> {
    let (continuous_dims, discrete_dims) = domain.fixed_rectangular_dims().ok_or_else(|| {
        BuildError::incompatible("plane geometry requires a fixed rectangular domain")
    })?;
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != continuous_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != discrete_dims {
        return Err(BuildError::incompatible(format!(
            "plane geometry discrete dimension mismatch: expected {}, got {}",
            discrete_dims,
            geometry.discrete.len()
        )));
    }
    Ok(())
}

fn validate_line_geometry(
    geometry: &LineRasterGeometry,
    domain: &Domain,
) -> Result<(), BuildError> {
    let (continuous_dims, discrete_dims) = domain.fixed_rectangular_dims().ok_or_else(|| {
        BuildError::incompatible("line geometry requires a fixed rectangular domain")
    })?;
    geometry.validate().map_err(BuildError::invalid_input)?;
    if geometry.offset.len() != continuous_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry continuous dimension mismatch: expected {}, got {}",
            continuous_dims,
            geometry.offset.len()
        )));
    }
    if geometry.discrete.len() != discrete_dims {
        return Err(BuildError::incompatible(format!(
            "line geometry discrete dimension mismatch: expected {}, got {}",
            discrete_dims,
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

fn permuted_raster_index(index: usize, total_samples: usize, stride: usize) -> usize {
    if total_samples <= 1 {
        return index.min(total_samples.saturating_sub(1));
    }
    (index * stride) % total_samples
}

fn coprime_stride(total_samples: usize) -> usize {
    if total_samples <= 1 {
        return 1;
    }

    let phi_minus_one = 0.618_033_988_749_894_9_f64;
    let mut candidate =
        ((total_samples as f64 * phi_minus_one).floor() as usize).clamp(1, total_samples - 1);
    while candidate.gcd(&total_samples) != 1 {
        candidate += 1;
        if candidate >= total_samples {
            candidate = 1;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::{
        RasterLineSampler, RasterLineSamplerParams, coprime_stride, permuted_raster_index,
    };
    use crate::core::{LineRasterGeometry, Linspace};
    use crate::sampling::SamplerAggregator;
    use crate::utils::domain::Domain;
    use num::Integer;

    #[test]
    fn permuted_raster_index_visits_each_sample_once() {
        for total_samples in 1..32 {
            let stride = coprime_stride(total_samples);
            let mut seen = vec![false; total_samples];
            for index in 0..total_samples {
                let permuted = permuted_raster_index(index, total_samples, stride);
                assert!(permuted < total_samples);
                assert!(!seen[permuted], "duplicate index for n={total_samples}");
                seen[permuted] = true;
            }
            assert!(seen.into_iter().all(|value| value));
            assert_eq!(coprime_stride(total_samples).gcd(&total_samples), 1);
        }
    }

    #[test]
    fn raster_line_snapshot_restores_shuffled_progress() {
        let domain = Domain::rectangular(1, 0);
        let params = RasterLineSamplerParams {
            geometry: LineRasterGeometry {
                offset: vec![0.0],
                direction: vec![1.0],
                linspace: Linspace {
                    start: 0.0,
                    stop: 4.0,
                    count: 5,
                },
                discrete: Vec::new(),
            },
        };
        let mut sampler = RasterLineSampler::from_params_and_domain(params.clone(), &domain)
            .expect("build sampler");
        let first_batch = sampler.produce_latent_batch(2).expect("first batch");
        let snapshot = sampler.snapshot().expect("snapshot");
        let restored_snapshot = match snapshot {
            crate::sampling::SamplerAggregatorSnapshot::RasterLine { raw } => {
                serde_json::from_value(raw).expect("decode raster line snapshot")
            }
            other => panic!("unexpected snapshot kind: {other:?}"),
        };
        let mut restored =
            RasterLineSampler::from_snapshot(restored_snapshot, &domain).expect("restore");
        let second_batch = restored.produce_latent_batch(3).expect("second batch");

        let first_batch = first_batch.payload.as_batch().expect("decode first batch");
        let second_batch = second_batch
            .payload
            .as_batch()
            .expect("decode second batch");
        let first_points = first_batch
            .points()
            .iter()
            .map(|point| point.continuous[0])
            .collect::<Vec<_>>();
        let second_points = second_batch
            .points()
            .iter()
            .map(|point| point.continuous[0])
            .collect::<Vec<_>>();

        assert_eq!(first_points, vec![0.0, 3.0]);
        assert_eq!(second_points, vec![1.0, 4.0, 2.0]);
    }
}
