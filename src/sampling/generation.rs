//! Preserve sampler-call boundaries independently of evaluator work units.

use crate::core::EngineError;
use crate::sampling::LatentBatch;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct GenerationBuffer {
    pending: Option<BufferedDraw>,
    training_groups: VecDeque<TrainingGroup>,
    legacy_pending_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_training_groups_survive_checkpoint_roundtrips() {
        let mut state = GenerationBuffer::default();
        state.restore_legacy_samples(2);
        state.record_draw(5);
        state.record_draw(2);
        assert_eq!(
            state.accept_training_values(&[8.0, 9.0]).unwrap(),
            Some(vec![8.0, 9.0])
        );
        assert_eq!(state.accept_training_values(&[0.0, 1.0]).unwrap(), None);
        let encoded = serde_json::to_value(&state).unwrap();
        let mut restored: GenerationBuffer = serde_json::from_value(encoded).unwrap();
        restored.restore_legacy_samples(5);
        assert_eq!(
            restored.accept_training_values(&[2.0, 3.0, 4.0]).unwrap(),
            Some(vec![0.0, 1.0, 2.0, 3.0, 4.0])
        );
        assert!(restored.accept_training_values(&[5.0, 6.0, 7.0]).is_err());
        assert_eq!(
            restored.accept_training_values(&[5.0, 6.0]).unwrap(),
            Some(vec![5.0, 6.0])
        );
        assert!(restored.is_empty());
    }

    #[test]
    fn old_checkpoint_progress_keeps_its_serialized_shape() {
        let old = serde_json::json!({
            "produced_batches_total": 3, "produced_samples_total": 48,
            "ingested_batches_total": 1, "ingested_samples_total": 16,
            "sampler_uptime_ms_accumulated": 0.0, "accumulator_checkpoint_state": "Ready",
        });
        let restored: crate::core::checkpoint::SamplerProgress =
            serde_json::from_value(old.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), old);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BufferedDraw {
    batch: LatentBatch,
    next_sample: usize,
    training_remaining: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrainingGroup {
    samples: usize,
    values: Vec<f64>,
}

impl GenerationBuffer {
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn training_remaining_at_draw(&self) -> Option<usize> {
        self.pending
            .as_ref()
            .and_then(|draw| draw.training_remaining)
    }

    pub fn buffer_draw(
        &mut self,
        batch: LatentBatch,
        training_remaining: Option<usize>,
    ) -> Result<(), EngineError> {
        batch
            .validate_nr_samples()
            .map_err(|err| EngineError::engine(err.to_string()))?;
        self.pending = Some(BufferedDraw {
            batch,
            next_sample: 0,
            training_remaining,
        });
        Ok(())
    }

    pub fn take_batches(
        &mut self,
        slots: usize,
        chunk_size: usize,
    ) -> Result<Vec<LatentBatch>, EngineError> {
        if chunk_size == 0 {
            return Err(EngineError::engine("evaluator chunk size is zero"));
        }
        let Some(draw) = self.pending.as_mut() else {
            return Ok(Vec::new());
        };
        draw.batch
            .validate_nr_samples()
            .map_err(|err| EngineError::engine(err.to_string()))?;
        if draw.next_sample >= draw.batch.nr_samples {
            return Err(EngineError::engine("invalid buffered generation cursor"));
        }
        let mut batches = Vec::new();
        for _ in 0..slots {
            let size = chunk_size.min(draw.batch.nr_samples - draw.next_sample);
            if size == 0 {
                break;
            }
            let batch = draw
                .batch
                .slice(draw.next_sample, size)
                .map_err(|err| EngineError::engine(err.to_string()))?;
            draw.next_sample += size;
            batches.push(batch);
        }
        if draw.next_sample == draw.batch.nr_samples {
            self.pending = None;
        }
        Ok(batches)
    }
    pub fn pending_samples(&self) -> usize {
        self.pending.as_ref().map_or(0, |draw| {
            draw.batch.nr_samples.saturating_sub(draw.next_sample)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_none()
            && self.training_groups.is_empty()
            && self.legacy_pending_samples == 0
    }

    /// Old checkpoints have no generation metadata. Their already queued work
    /// still has one sampler call per evaluator batch and must be ingested first.
    pub fn restore_legacy_samples(&mut self, queued_samples: usize) {
        let grouped_samples: usize = self
            .training_groups
            .iter()
            .map(|group| group.samples - group.values.len())
            .sum();
        self.legacy_pending_samples =
            (queued_samples + self.pending_samples()).saturating_sub(grouped_samples);
    }

    pub fn record_draw(&mut self, samples: usize) {
        self.training_groups.push_back(TrainingGroup {
            samples,
            values: Vec::new(),
        });
    }

    /// Ordered evaluator results cannot cross a generation boundary. Return
    /// exactly one original draw's weights once all its fragments have arrived.
    pub fn accept_training_values(
        &mut self,
        values: &[f64],
    ) -> Result<Option<Vec<f64>>, EngineError> {
        if self.legacy_pending_samples > 0 {
            self.legacy_pending_samples = self
                .legacy_pending_samples
                .checked_sub(values.len())
                .ok_or_else(|| {
                    EngineError::engine("training result crosses a legacy generation boundary")
                })?;
            return Ok(Some(values.to_vec()));
        }
        let group = self.training_groups.front_mut().ok_or_else(|| {
            EngineError::engine("training result has no matching generated batch")
        })?;
        if values.len() > group.samples - group.values.len() {
            return Err(EngineError::engine(
                "training result crosses a generation boundary",
            ));
        }
        group.values.extend_from_slice(values);
        if group.values.len() == group.samples {
            Ok(Some(self.training_groups.pop_front().unwrap().values))
        } else {
            Ok(None)
        }
    }
}
