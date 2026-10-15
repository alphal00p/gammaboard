//! Preserve sampler-call boundaries independently of evaluator work units.

use crate::core::EngineError;
use crate::sampling::LatentBatch;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct GenerationBuffer {
    pending: Option<BufferedDraw>,
    training_groups: VecDeque<TrainingGroup>,
    pub finished: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_training_groups_survive_checkpoint_roundtrips() {
        let mut state = GenerationBuffer::default();
        state.record_draw(5);
        state.record_draw(2);
        assert_eq!(state.accept_training_values(&[0.0, 1.0]).unwrap(), None);
        let encoded = serde_json::to_value(&state).unwrap();
        let mut restored: GenerationBuffer = serde_json::from_value(encoded).unwrap();
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
    fn ragged_draw_restores_its_split_cursor_and_feedback_metadata() {
        use crate::evaluation::{Batch, Point};
        use crate::sampling::LatentBatch;
        let original = Batch::new(
            (0..9)
                .map(|i| Point::new(vec![i as f64; i % 3 + 1], vec![(i % 2) as i64], 1.0))
                .collect(),
        );
        let mut state = GenerationBuffer::default();
        state
            .buffer_draw(LatentBatch::from_batch(&original), Some(12))
            .unwrap();
        state.record_draw(9);
        let mut chunks = state.take_batches(2, 2).unwrap();
        assert!(state.accept_training_values(&[0.0, 1.0]).unwrap().is_none());
        let snapshot = serde_json::to_value(state).unwrap();
        let mut restored: GenerationBuffer = serde_json::from_value(snapshot).unwrap();
        assert!(restored.requires_feedback());
        assert_eq!(restored.training_remaining(), Some(8));
        chunks.extend(restored.take_batches(10, 3).unwrap());
        let recovered: Vec<_> = chunks
            .into_iter()
            .flat_map(|chunk| chunk.payload.into_batch().unwrap().points().to_vec())
            .collect();
        assert_eq!(recovered, original.points());
        assert!(
            restored
                .accept_training_values(&[2.0, 3.0])
                .unwrap()
                .is_none()
        );
        assert!(
            restored
                .accept_training_values(&[4.0, 5.0, 6.0])
                .unwrap()
                .is_none()
        );
        assert_eq!(
            restored.accept_training_values(&[7.0, 8.0]).unwrap(),
            Some((0..9).map(|i| i as f64).collect())
        );
        assert!(!restored.has_outstanding());
        restored.finished = true;
        assert!(
            !restored.is_empty(),
            "the finished marker must survive serialization"
        );
        let restored: GenerationBuffer =
            serde_json::from_value(serde_json::to_value(restored).unwrap()).unwrap();
        assert!(restored.finished);
        assert!(!restored.has_outstanding());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BufferedDraw {
    batch: LatentBatch,
    next_sample: usize,
    #[serde(skip)]
    next_coordinate: Option<usize>,
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

    pub fn training_remaining(&self) -> Option<usize> {
        self.pending.as_ref().and_then(|draw| {
            draw.training_remaining
                .map(|remaining| remaining.saturating_sub(draw.next_sample))
        })
    }

    pub fn requires_feedback(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|draw| draw.training_remaining.is_some())
    }

    pub fn buffer_draw(
        &mut self,
        batch: LatentBatch,
        training_remaining: Option<usize>,
    ) -> Result<(), EngineError> {
        if self.pending.is_some() {
            return Err(EngineError::engine("cannot overwrite an undispatched draw"));
        }
        batch
            .validate_nr_samples()
            .map_err(|err| EngineError::engine(err.to_string()))?;
        self.pending = Some(BufferedDraw {
            batch,
            next_sample: 0,
            next_coordinate: Some(0),
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
        // Validate restored buffers once; newly generated buffers are already checked.
        if draw.next_coordinate.is_none() {
            draw.batch
                .validate_nr_samples()
                .map_err(|err| EngineError::engine(err.to_string()))?;
            draw.next_coordinate = Some(match &draw.batch.payload {
                crate::sampling::LatentBatchPayload::IndexedBatch {
                    continuous_layouts, ..
                } => continuous_layouts
                    .get(..draw.next_sample)
                    .ok_or_else(|| EngineError::engine("invalid buffered generation cursor"))?
                    .iter()
                    .sum(),
                _ => 0,
            });
        }
        if slots > 0 && draw.next_sample == 0 && chunk_size >= draw.batch.nr_samples {
            return Ok(vec![self.pending.take().unwrap().batch]);
        }
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
                .slice_at(draw.next_sample, size, draw.next_coordinate.unwrap())
                .map_err(|err| EngineError::engine(err.to_string()))?;
            if let crate::sampling::LatentBatchPayload::IndexedBatch {
                continuous_values, ..
            } = &batch.payload
            {
                *draw.next_coordinate.as_mut().unwrap() += continuous_values.len();
            }
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
        !self.finished && !self.has_outstanding()
    }

    pub fn has_outstanding(&self) -> bool {
        self.pending.is_some() || !self.training_groups.is_empty()
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
