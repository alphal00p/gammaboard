use rand::{RngCore, SeedableRng};
use rand_xoshiro::Xoshiro256StarStar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableMonteCarloRng {
    state: Xoshiro256StarStar,
}

impl SerializableMonteCarloRng {
    pub fn new(seed: u64, stream_id: usize) -> Self {
        let mut state = Xoshiro256StarStar::seed_from_u64(seed);
        for _ in 0..stream_id {
            state.jump();
        }
        Self { state }
    }
}

impl RngCore for SerializableMonteCarloRng {
    fn next_u32(&mut self) -> u32 {
        self.state.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.state.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.state.fill_bytes(dest);
    }
}
