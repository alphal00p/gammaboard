use rand::random;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const BATCH_ID_COUNTER_BITS: u32 = 6;
const BATCH_ID_PROCESS_ENTROPY_BITS: u32 = 16;
const BATCH_ID_SUFFIX_BITS: u32 = BATCH_ID_COUNTER_BITS + BATCH_ID_PROCESS_ENTROPY_BITS;
const BATCH_ID_COUNTER_MASK: u16 = (1u16 << BATCH_ID_COUNTER_BITS) - 1;

#[derive(Debug)]
struct BatchIdGenerator {
    last_millis: u64,
    counter: u16,
    process_entropy: u16,
}

impl BatchIdGenerator {
    fn new() -> Self {
        Self {
            last_millis: 0,
            counter: 0,
            process_entropy: random::<u16>(),
        }
    }

    fn next_ids(&mut self, count: usize) -> Vec<i64> {
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let now_millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_millis() as u64;
            if now_millis > self.last_millis {
                self.last_millis = now_millis;
                self.counter = 0;
            } else if self.counter >= BATCH_ID_COUNTER_MASK {
                self.last_millis = self.last_millis.saturating_add(1);
                self.counter = 0;
            }

            let id = ((self.last_millis as i64) << BATCH_ID_SUFFIX_BITS)
                | ((self.process_entropy as i64) << BATCH_ID_COUNTER_BITS)
                | (self.counter as i64);
            self.counter = self.counter.saturating_add(1);
            ids.push(id);
        }
        ids
    }
}

pub fn next_batch_ids(count: usize) -> Vec<i64> {
    static GENERATOR: OnceLock<Mutex<BatchIdGenerator>> = OnceLock::new();
    let generator = GENERATOR.get_or_init(|| Mutex::new(BatchIdGenerator::new()));
    generator
        .lock()
        .expect("batch id generator lock poisoned")
        .next_ids(count)
}
