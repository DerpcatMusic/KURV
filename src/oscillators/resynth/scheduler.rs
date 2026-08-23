//! Bounded block scheduling and fixed-capacity grain admission.
//!
//! This module contains only deterministic control-plane math. It does not
//! allocate, lock, or inspect audio samples, so the renderer can use it from
//! a realtime callback after preparation has completed.

pub const INTERNAL_GRAIN_CAPACITY: usize = 64;
pub const MAX_BLOCK_SPAWNS: usize = 256;
const EMPTY: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensityPlan {
    pub requested_rate_hz: f32,
    pub effective_rate_hz: f32,
    pub requested_overlap: f32,
    pub effective_overlap: f32,
}

#[must_use]
pub fn density_plan(rate_hz: f32, duration_seconds: f32, capacity: usize) -> DensityPlan {
    let rate = if rate_hz.is_finite() {
        rate_hz.max(0.0)
    } else {
        0.0
    };
    let duration = if duration_seconds.is_finite() {
        duration_seconds.max(0.0)
    } else {
        0.0
    };
    let requested_overlap = rate * duration;
    let capacity = capacity as f32;
    let effective_rate_hz = if duration > 0.0 {
        rate.min(capacity / duration)
    } else {
        rate
    };
    DensityPlan {
        requested_rate_hz: rate,
        effective_rate_hz,
        requested_overlap,
        effective_overlap: effective_rate_hz * duration,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnSchedule {
    offsets: [u16; MAX_BLOCK_SPAWNS],
    count: u16,
}

impl Default for SpawnSchedule {
    fn default() -> Self {
        Self {
            offsets: [0; MAX_BLOCK_SPAWNS],
            count: 0,
        }
    }
}

impl SpawnSchedule {
    #[must_use]
    pub fn as_slice(&self) -> &[u16] {
        &self.offsets[..usize::from(self.count).min(MAX_BLOCK_SPAWNS)]
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.count as usize
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockScheduler {
    samples_until_spawn: f32,
}

impl BlockScheduler {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn schedule_block(
        &mut self,
        rate_hz: f32,
        duration_seconds: f32,
        sample_rate: f32,
        block_len: usize,
        capacity: usize,
    ) -> (DensityPlan, SpawnSchedule) {
        let plan = density_plan(rate_hz, duration_seconds, capacity);
        let mut schedule = SpawnSchedule::default();
        let sample_rate = if sample_rate.is_finite() {
            sample_rate.max(1.0)
        } else {
            1.0
        };
        let period = if plan.effective_rate_hz > 0.0 {
            sample_rate / plan.effective_rate_hz
        } else {
            f32::INFINITY
        };
        let block_len_f32 = block_len as f32;
        if !period.is_finite() || period <= 0.0 {
            // Disabled density must not poison the phase state. A later
            // positive-rate block starts deterministically at offset zero.
            self.samples_until_spawn = 0.0;
            return (plan, schedule);
        }
        self.samples_until_spawn = self.samples_until_spawn.max(0.0);
        while self.samples_until_spawn < block_len_f32
            && usize::from(schedule.count) < MAX_BLOCK_SPAWNS
        {
            schedule.offsets[usize::from(schedule.count)] =
                self.samples_until_spawn
                    .floor()
                    .clamp(0.0, f32::from(u16::MAX)) as u16;
            schedule.count += 1;
            self.samples_until_spawn += period;
        }
        self.samples_until_spawn -= block_len_f32;
        (plan, schedule)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrainHandle {
    index: u16,
    generation: u32,
}

impl GrainHandle {
    #[must_use]
    pub const fn index(self) -> usize {
        self.index as usize
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    value: u32,
    active: bool,
    generation: u32,
    deadline: u64,
    free_next: u16,
    bucket_prev: u16,
    bucket_next: u16,
    bucket: u16,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            value: 0,
            active: false,
            generation: 0,
            deadline: 0,
            free_next: EMPTY,
            bucket_prev: EMPTY,
            bucket_next: EMPTY,
            bucket: 0,
        }
    }
}

/// Fixed-capacity grain metadata arena.
///
/// Admission and explicit release use a free list and doubly-linked expiry
/// buckets. Neither operation scans the active set. `expire_one` advances one
/// wheel bucket and returns one due handle per call.
#[derive(Clone, Copy, Debug)]
pub struct GrainArena<const CAPACITY: usize, const BUCKETS: usize> {
    slots: [Slot; CAPACITY],
    bucket_heads: [u16; BUCKETS],
    free_head: u16,
    active_count: usize,
}

impl<const CAPACITY: usize, const BUCKETS: usize> Default for GrainArena<CAPACITY, BUCKETS> {
    fn default() -> Self {
        let mut slots = [Slot::default(); CAPACITY];
        let mut index = 0;
        while index < CAPACITY {
            slots[index].free_next = if index + 1 < CAPACITY {
                index as u16 + 1
            } else {
                EMPTY
            };
            index += 1;
        }
        Self {
            slots,
            bucket_heads: [EMPTY; BUCKETS],
            free_head: if CAPACITY == 0 { EMPTY } else { 0 },
            active_count: 0,
        }
    }
}

impl<const CAPACITY: usize, const BUCKETS: usize> GrainArena<CAPACITY, BUCKETS> {
    const VALIDATE_CAPACITY: () =
        assert!(CAPACITY <= u16::MAX as usize && BUCKETS <= u16::MAX as usize);

    #[must_use]
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    #[must_use]
    pub const fn active_count(&self) -> usize {
        self.active_count
    }

    #[must_use]
    pub fn admit(&mut self, value: u32, deadline: u64) -> Option<GrainHandle> {
        if self.free_head == EMPTY || BUCKETS == 0 {
            return None;
        }
        let index = usize::from(self.free_head);
        let next_free = self.slots[index].free_next;
        let generation = self.slots[index].generation.wrapping_add(1).max(1);
        let bucket = (deadline as usize % BUCKETS) as u16;
        let bucket_next = self.bucket_heads[usize::from(bucket)];
        self.free_head = next_free;
        self.slots[index] = Slot {
            value,
            active: true,
            generation,
            deadline,
            free_next: EMPTY,
            bucket_prev: EMPTY,
            bucket_next,
            bucket,
        };
        if bucket_next != EMPTY {
            self.slots[usize::from(bucket_next)].bucket_prev = index as u16;
        }
        self.bucket_heads[usize::from(bucket)] = index as u16;
        self.active_count += 1;
        Some(GrainHandle {
            index: index as u16,
            generation,
        })
    }

    #[must_use]
    pub fn value(&self, handle: GrainHandle) -> Option<u32> {
        self.slots
            .get(handle.index())
            .filter(|slot| slot.active && slot.generation == handle.generation)
            .map(|slot| slot.value)
    }

    pub fn release(&mut self, handle: GrainHandle) -> Option<u32> {
        if handle.index() >= CAPACITY
            || !self.slots[handle.index()].active
            || self.slots[handle.index()].generation != handle.generation
        {
            return None;
        }
        let value = self.unlink_active(handle);
        self.push_free(handle);
        Some(value)
    }

    /// Return and release one grain whose deadline is at or before `now`.
    ///
    /// The wheel is scanned by bucket rather than by sample tick, so a host
    /// block that advances time by many samples cannot strand an overdue slot.
    pub fn expire_one(&mut self, now: u64) -> Option<(GrainHandle, u32)> {
        if BUCKETS == 0 {
            return None;
        }
        for bucket in 0..BUCKETS {
            let mut cursor = self.bucket_heads[bucket];
            while cursor != EMPTY {
                let next = self.slots[usize::from(cursor)].bucket_next;
                if self.slots[usize::from(cursor)].deadline <= now {
                    let handle = GrainHandle {
                        index: cursor,
                        generation: self.slots[usize::from(cursor)].generation,
                    };
                    let value = self.unlink_active(handle);
                    self.push_free(handle);
                    return Some((handle, value));
                }
                cursor = next;
            }
        }
        None
    }

    fn unlink_bucket(&mut self, handle: GrainHandle) {
        let index = handle.index();
        let slot = self.slots[index];
        if slot.bucket_prev == EMPTY {
            self.bucket_heads[usize::from(slot.bucket)] = slot.bucket_next;
        } else {
            self.slots[usize::from(slot.bucket_prev)].bucket_next = slot.bucket_next;
        }
        if slot.bucket_next != EMPTY {
            self.slots[usize::from(slot.bucket_next)].bucket_prev = slot.bucket_prev;
        }
        self.slots[index].bucket_prev = EMPTY;
        self.slots[index].bucket_next = EMPTY;
    }

    fn unlink_active(&mut self, handle: GrainHandle) -> u32 {
        let index = handle.index();
        let value = self.slots[index].value;
        self.unlink_bucket(handle);
        self.slots[index].active = false;
        self.active_count -= 1;
        value
    }

    fn push_free(&mut self, handle: GrainHandle) {
        let index = handle.index();
        self.slots[index].free_next = self.free_head;
        self.free_head = handle.index;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_is_rate_times_complete_duration() {
        let plan = density_plan(2_000.0, 1.0, INTERNAL_GRAIN_CAPACITY);
        assert_eq!(plan.requested_overlap, 2_000.0);
        assert_eq!(plan.effective_rate_hz, INTERNAL_GRAIN_CAPACITY as f32);
        assert_eq!(plan.effective_overlap, INTERNAL_GRAIN_CAPACITY as f32);
    }

    #[test]
    fn density_never_exceeds_capacity_even_with_invalid_inputs() {
        let plan = density_plan(f32::NAN, f32::INFINITY, 64);
        assert_eq!(plan.requested_rate_hz, 0.0);
        assert_eq!(plan.effective_overlap, 0.0);
        let plan = density_plan(20.0, 0.1, 4);
        assert_eq!(plan.effective_rate_hz, 20.0);
    }

    #[test]
    fn block_schedule_reports_offsets_and_carries_fractional_remainder() {
        let mut scheduler = BlockScheduler::default();
        let (_, first) = scheduler.schedule_block(1_000.0, 0.01, 48_000.0, 64, 64);
        assert_eq!(first.as_slice(), &[0, 48]);
        let (_, second) = scheduler.schedule_block(1_000.0, 0.01, 48_000.0, 64, 64);
        assert_eq!(second.as_slice(), &[32]);
    }

    #[test]
    fn zero_density_recovers_on_the_next_enabled_block() {
        let mut scheduler = BlockScheduler::default();
        let (_, muted) = scheduler.schedule_block(0.0, 0.1, 48_000.0, 64, 64);
        assert!(muted.as_slice().is_empty());
        let (_, enabled) = scheduler.schedule_block(1_000.0, 0.01, 48_000.0, 64, 64);
        assert_eq!(enabled.as_slice(), &[0, 48]);
    }

    #[test]
    fn expiry_finds_overdue_grains_when_time_skips_wheel_buckets() {
        let mut arena = GrainArena::<2, 4>::default();
        let handle = arena.admit(42, 5).expect("grain");
        assert_eq!(arena.expire_one(8), Some((handle, 42)));
    }

    #[test]
    fn arena_admission_and_release_reuse_free_slots_without_stealing() {
        let mut arena = GrainArena::<2, 8>::default();
        let first = arena.admit(10, 20).expect("first slot");
        let second = arena.admit(20, 30).expect("second slot");
        assert!(arena.admit(30, 40).is_none());
        assert_eq!(arena.value(first), Some(10));
        assert_eq!(arena.release(first), Some(10));
        let reused = arena.admit(30, 40).expect("released slot");
        assert_eq!(reused.index(), first.index());
        assert_ne!(reused, first);
        assert_eq!(arena.value(first), None);
        assert_eq!(arena.value(second), Some(20));
    }

    #[test]
    fn expiry_is_deadline_ordered_and_ignores_future_wheel_collisions() {
        let mut arena = GrainArena::<4, 4>::default();
        let late = arena.admit(20, 9).expect("late");
        let early = arena.admit(10, 5).expect("early");
        assert_eq!(arena.expire_one(4), None);
        assert_eq!(arena.expire_one(5), Some((early, 10)));
        assert_eq!(arena.expire_one(8), None);
        assert_eq!(arena.expire_one(9), Some((late, 20)));
        assert_eq!(arena.active_count(), 0);
    }
}
