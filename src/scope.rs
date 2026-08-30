use std::sync::atomic::{AtomicU8, AtomicU32, Ordering, fence};

pub(crate) const SCOPE_SAMPLES: usize = 96;
const INTEREST_BLOCKS: u8 = 8;

pub(crate) struct ScopeTransport {
    interest: AtomicU8,
    generation: AtomicU32,
    samples: [AtomicU32; SCOPE_SAMPLES],
}

impl Default for ScopeTransport {
    fn default() -> Self {
        Self {
            interest: AtomicU8::new(0),
            generation: AtomicU32::new(0),
            samples: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
}

impl ScopeTransport {
    pub(crate) fn publish(&self, samples: &[f32]) {
        let interest = self.interest.load(Ordering::Acquire);
        if interest == 0 || samples.is_empty() {
            return;
        }
        self.interest.store(interest - 1, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::AcqRel);
        let last = samples.len() - 1;
        for (index, target) in self.samples.iter().enumerate() {
            let sample = samples[index * last / (SCOPE_SAMPLES - 1)];
            target.store(
                if sample.is_finite() { sample } else { 0.0 }.to_bits(),
                Ordering::Relaxed,
            );
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn snapshot(&self) -> Option<[f32; SCOPE_SAMPLES]> {
        self.interest.store(INTEREST_BLOCKS, Ordering::Release);
        for _ in 0..2 {
            let before = self.generation.load(Ordering::Acquire);
            if before == 0 || before & 1 != 0 {
                continue;
            }
            let samples = std::array::from_fn(|index| {
                f32::from_bits(self.samples[index].load(Ordering::Relaxed))
            });
            fence(Ordering::Acquire);
            if self.generation.load(Ordering::Relaxed) == before {
                return Some(samples);
            }
        }
        None
    }
}
