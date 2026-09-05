//! Per-item CPU attribution over time.
//!
//! Every performance decision in this crate so far has been argued from first
//! principles: the block fast paths are faster because they hoist work out of
//! the frame loop, the helper pool is faster because it uses another core, the
//! declick gate is cheap because nine samples is a small fraction of a block.
//! Those arguments are probably right. None of them has been measured on the
//! machine that actually runs the plugin.
//!
//! This module measures them. It records, per host process block, how long each
//! phase of the block took and how many samples took each route through the
//! renderer, and streams that series to disk so it can be read as a time series
//! rather than an average. Averages are the wrong summary for audio: a mean
//! block time well inside budget is perfectly compatible with a periodic spike
//! that xruns once a second, and the spike is the only part anybody hears.
//!
//! # Cost
//!
//! Disabled by default and disabled at zero cost: [`enabled`] is one relaxed
//! atomic load, and every entry point is `#[inline]` and returns immediately.
//! Enable it by setting `KURV_CPU_PROFILE` to the path of the CSV to write.
//!
//! When enabled, the cost is one `Instant::now()` per phase per block — roughly
//! eight clock reads for a block of hundreds of samples, which is far below the
//! noise floor of what it measures. Nothing here times anything inside a
//! per-sample loop, because a 25 ns clock read per sample would cost more than
//! most of the stages being measured.
//!
//! That constraint is why the timed items stop at [`Item::Render`] rather than
//! splitting it into oscillators, filters and oversampling. Those stages are
//! genuinely interleaved inside the chunk loop — a chunk fills modulation,
//! renders voices, pushes the oversampler and drains it, all before the next
//! chunk starts — so there is no contiguous region to bracket. Splitting them
//! honestly needs per-chunk brackets threaded through every render branch, and
//! until that exists this module reports the render loop as one item rather
//! than reporting a split it cannot actually measure.
//!
//! Attribution inside the render loop is done with [`BlockProfile::count`]
//! instead: the renderer says how many host frames went down each route, and
//! per-route cost is recovered by regressing block time against route counts
//! across many blocks rather than by timing any one of them.
//!
//! # Real-time safety
//!
//! The audio thread only ever writes to a preallocated fixed-capacity ring and
//! bumps two atomics. It never allocates, never locks, and never blocks. If the
//! ring is full — the writer thread fell behind, or was descheduled — the audio
//! thread drops the frame and increments a drop counter, so a slow consumer
//! costs measurements rather than audio.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Instant;

/// A profiled item: either a phase of the block or a route through it.
///
/// Phases are timed. Routes are counted, because they live inside per-sample
/// loops where a clock read would dominate the thing it measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Item {
    /// Whole `process` call, wall clock. The budget every other item spends.
    Block = 0,
    /// Incoming parameter modulation and note events.
    Events = 1,
    /// Reconciling the generator topology, oscillator, filter, unison and group
    /// configuration against the parameter snapshots. All of it runs whether or
    /// not anything changed, so it is a fixed per-block tax and worth knowing
    /// the size of.
    Configuration = 2,
    /// The whole render loop: modulation, voices, oversampling, filters.
    Render = 3,
    /// Peak metering, meter publication and resynth telemetry.
    Metering = 4,

    /// Host frames rendered by a block-major path.
    RouteBlockMajor = 5,
    /// Host frames rendered one at a time by the serial path.
    RouteSerial = 6,
    /// Host frames the serial path had to take because a declick residual was
    /// still draining. The measured cost of the gate introduced to keep the
    /// block renderers out of the transient window.
    RouteDeclickGated = 7,
    /// Chunks handed to the internal helper pool and accepted.
    PoolAccepted = 8,
    /// Chunks the helper pool declined or failed to deliver before its
    /// deadline, and which the calling thread therefore rendered itself.
    PoolFallback = 9,
}

impl Item {
    /// Number of distinct items. Sized by the last discriminant.
    pub(crate) const COUNT: usize = 10;

    /// Column name for this item in the CSV.
    const fn label(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Events => "events",
            Self::Configuration => "configuration",
            Self::Render => "render",
            Self::Metering => "metering",
            Self::RouteBlockMajor => "route_block_major",
            Self::RouteSerial => "route_serial",
            Self::RouteDeclickGated => "route_declick_gated",
            Self::PoolAccepted => "pool_accepted",
            Self::PoolFallback => "pool_fallback",
        }
    }

    /// Every item, in discriminant order, for header emission.
    const ALL: [Self; Self::COUNT] = [
        Self::Block,
        Self::Events,
        Self::Configuration,
        Self::Render,
        Self::Metering,
        Self::RouteBlockMajor,
        Self::RouteSerial,
        Self::RouteDeclickGated,
        Self::PoolAccepted,
        Self::PoolFallback,
    ];

    /// Whether the item is a timed phase rather than a counted route.
    const fn timed(self) -> bool {
        (self as u8) <= (Self::Metering as u8)
    }
}

/// One process block's worth of measurements.
///
/// Nanoseconds are `u32`, which saturates at 4.29 seconds. A process block that
/// takes four seconds has problems this module is not going to help with.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    /// Monotonic block index since the profile was enabled.
    pub(crate) index: u64,
    /// Host frames in the block.
    pub(crate) host_frames: u32,
    /// Oversampling factor in force for the block.
    pub(crate) factor: u8,
    /// Voices sounding at the end of the block.
    pub(crate) voices: u8,
    /// Elapsed nanoseconds per timed item, sample counts per counted item.
    pub(crate) values: [u32; Item::COUNT],
}

impl Default for Frame {
    fn default() -> Self {
        Self { index: 0, host_frames: 0, factor: 1, voices: 0, values: [0; Item::COUNT] }
    }
}

/// Ring capacity, in frames.
///
/// At a 64-sample block and 48 kHz the audio thread produces 750 frames per
/// second, so this is about five seconds of slack for the writer thread. That
/// is more than enough to ride out a page fault or a scheduler hiccup, and it
/// is 30 KiB, which is nothing.
const RING_CAPACITY: usize = 4096;

struct Ring {
    slots: Box<[std::cell::UnsafeCell<Frame>]>,
    /// Next slot the producer will write. Only the audio thread stores here.
    head: AtomicUsize,
    /// Next slot the consumer will read. Only the writer thread stores here.
    tail: AtomicUsize,
    /// Frames the audio thread dropped because the ring was full.
    dropped: AtomicU32,
}

// SAFETY: the `UnsafeCell` contents are only accessed by one thread at a time.
// The producer writes a slot before publishing it with a `Release` store to
// `head`; the consumer only reads slots strictly below the `Acquire`-loaded
// `head`, and only frees them with a `Release` store to `tail`. The producer
// only writes slots at or above the `Acquire`-loaded `tail`.
unsafe impl Sync for Ring {}

impl Ring {
    fn new() -> Self {
        Self {
            slots: (0..RING_CAPACITY)
                .map(|_| std::cell::UnsafeCell::new(Frame::default()))
                .collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            dropped: AtomicU32::new(0),
        }
    }

    /// Publish a frame. Returns false if the ring was full and it was dropped.
    ///
    /// Called only from the audio thread. Wait-free: no allocation, no lock, no
    /// unbounded loop.
    fn push(&self, frame: Frame) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= RING_CAPACITY {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // SAFETY: the slot is strictly ahead of `tail`, so the consumer will
        // not touch it until the `Release` store below publishes it.
        unsafe { *self.slots[head % RING_CAPACITY].get() = frame };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Take the next frame, if the producer has published one.
    ///
    /// Called only from the writer thread.
    fn pop(&self) -> Option<Frame> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the slot is strictly below `head`, so the producer published
        // it and will not touch it again until we release it below.
        let frame = unsafe { *self.slots[tail % RING_CAPACITY].get() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(frame)
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static RING: std::sync::OnceLock<Ring> = std::sync::OnceLock::new();

/// Whether profiling is on.
///
/// One relaxed load. This is the entire cost of a disabled profile, and it is
/// what makes it safe to leave the instrumentation in the shipping build.
#[inline]
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn ring() -> &'static Ring {
    RING.get_or_init(Ring::new)
}

/// Turn profiling on and start the writer thread, if `KURV_CPU_PROFILE` names a
/// path.
///
/// Idempotent and safe to call from plugin initialization. Failing to open the
/// file leaves profiling off rather than failing the plugin: a diagnostic that
/// prevents the instrument from loading is worse than no diagnostic.
pub(crate) fn initialize() {
    let Ok(path) = std::env::var("KURV_CPU_PROFILE") else {
        return;
    };
    if path.is_empty() || ENABLED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(file) = File::create(&path) else {
        ENABLED.store(false, Ordering::Release);
        return;
    };
    std::thread::Builder::new()
        .name("kurv-cpu-profile".to_owned())
        .spawn(move || writer_loop(file))
        .map_or_else(|_| ENABLED.store(false, Ordering::Release), |_| ());
}

fn writer_loop(file: File) {
    let mut out = BufWriter::new(file);
    let mut header = String::from("index,host_frames,factor,voices");
    for item in Item::ALL {
        header.push(',');
        header.push_str(item.label());
        header.push_str(if item.timed() { "_ns" } else { "_n" });
    }
    if writeln!(out, "{header}").is_err() {
        return;
    }
    let mut line = String::with_capacity(256);
    loop {
        let mut wrote = false;
        while let Some(frame) = ring().pop() {
            line.clear();
            use std::fmt::Write as _;
            let _ = write!(
                line,
                "{},{},{},{}",
                frame.index, frame.host_frames, frame.factor, frame.voices
            );
            for value in frame.values {
                let _ = write!(line, ",{value}");
            }
            if writeln!(out, "{line}").is_err() {
                return;
            }
            wrote = true;
        }
        // Flush on the quiet edge rather than per frame, so the profile is
        // readable while it is being written without one syscall per block.
        if wrote && out.flush().is_err() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Frames the audio thread dropped because the writer fell behind.
///
/// A nonzero value means the profile has gaps, not that audio glitched.
#[cfg(test)]
pub(crate) fn dropped() -> u32 {
    ring().dropped.load(Ordering::Relaxed)
}

/// Turn profiling on for a test, without a writer thread or a file.
///
/// Frames accumulate in the ring until [`drain_for_test`] takes them.
#[cfg(test)]
pub(crate) fn enable_for_test() {
    ENABLED.store(true, Ordering::Release);
}

/// Turn profiling back off.
#[cfg(test)]
pub(crate) fn disable_for_test() {
    ENABLED.store(false, Ordering::Release);
}

/// Take every frame the ring currently holds.
#[cfg(test)]
pub(crate) fn drain_for_test() -> Vec<Frame> {
    std::iter::from_fn(|| ring().pop()).collect()
}

/// Accumulator for one process block.
///
/// Lives on the stack of `process`. When profiling is off every method is a
/// predictable-branch no-op and the struct itself is dead weight the optimizer
/// removes.
pub(crate) struct BlockProfile {
    active: bool,
    started: Option<Instant>,
    phase: Option<(Item, Instant)>,
    frame: Frame,
}

static BLOCK_INDEX: AtomicUsize = AtomicUsize::new(0);

impl BlockProfile {
    /// Begin measuring a process block.
    #[inline]
    pub(crate) fn begin(host_frames: usize, factor: u8) -> Self {
        if !enabled() {
            return Self { active: false, started: None, phase: None, frame: Frame::default() };
        }
        let index = BLOCK_INDEX.fetch_add(1, Ordering::Relaxed) as u64;
        Self {
            active: true,
            started: Some(Instant::now()),
            phase: None,
            frame: Frame {
                index,
                host_frames: u32::try_from(host_frames).unwrap_or(u32::MAX),
                factor,
                ..Frame::default()
            },
        }
    }

    /// Close the open phase, if any, and open `item`.
    ///
    /// Phases are exclusive by construction, so they sum to no more than
    /// [`Item::Block`] and the shortfall is honest unattributed time rather
    /// than double counting.
    #[inline]
    pub(crate) fn enter(&mut self, item: Item) {
        if !self.active {
            return;
        }
        debug_assert!(item.timed(), "{} is a counted route, not a phase", item.label());
        let now = Instant::now();
        self.close(now);
        self.phase = Some((item, now));
    }

    /// Close the open phase without opening another.
    #[inline]
    pub(crate) fn leave(&mut self) {
        if !self.active {
            return;
        }
        let now = Instant::now();
        self.close(now);
    }

    #[inline]
    fn close(&mut self, now: Instant) {
        if let Some((open, since)) = self.phase.take() {
            let elapsed = u32::try_from(now.duration_since(since).as_nanos()).unwrap_or(u32::MAX);
            let slot = &mut self.frame.values[open as usize];
            *slot = slot.saturating_add(elapsed);
        }
    }

    /// Add `n` to a counted route.
    ///
    /// Cheap enough to call from inside a per-sample loop, though calling it
    /// once with the loop length is cheaper still and just as informative.
    #[inline]
    pub(crate) fn count(&mut self, item: Item, n: u32) {
        if !self.active {
            return;
        }
        debug_assert!(!item.timed(), "{} is a timed phase, not a route", item.label());
        let slot = &mut self.frame.values[item as usize];
        *slot = slot.saturating_add(n);
    }

    /// Finish the block and publish it.
    #[inline]
    pub(crate) fn finish(mut self, voices: u8) {
        if !self.active {
            return;
        }
        let now = Instant::now();
        self.close(now);
        if let Some(started) = self.started {
            self.frame.values[Item::Block as usize] =
                u32::try_from(now.duration_since(started).as_nanos()).unwrap_or(u32::MAX);
        }
        self.frame.voices = voices;
        ring().push(self.frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every item must have a distinct slot, or two items silently share a
    /// column and every reading from both is wrong.
    #[test]
    fn item_discriminants_are_dense_and_distinct() {
        for (index, item) in Item::ALL.into_iter().enumerate() {
            assert_eq!(item as usize, index, "{} is out of order", item.label());
        }
        assert_eq!(Item::ALL.len(), Item::COUNT);
    }

    /// The timed/counted split is what decides whether a column is nanoseconds
    /// or a sample count, so a misclassified item mislabels its own units.
    #[test]
    fn timed_items_precede_counted_items() {
        let first_counted =
            Item::ALL.iter().position(|item| !item.timed()).expect("some item is counted");
        assert!(Item::ALL[..first_counted].iter().all(|item| item.timed()));
        assert!(Item::ALL[first_counted..].iter().all(|item| !item.timed()));
    }

    #[test]
    fn ring_preserves_order_and_reports_full() {
        let ring = Ring::new();
        for index in 0..RING_CAPACITY {
            let mut frame = Frame::default();
            frame.index = index as u64;
            assert!(ring.push(frame), "ring rejected a frame while it had room");
        }
        assert!(!ring.push(Frame::default()), "full ring accepted a frame");
        assert_eq!(ring.dropped.load(Ordering::Relaxed), 1);
        for index in 0..RING_CAPACITY {
            assert_eq!(ring.pop().expect("published frame").index, index as u64);
        }
        assert!(ring.pop().is_none(), "drained ring returned a frame");
    }

    /// A full ring that is then drained has to keep working, because the audio
    /// thread never stops producing and a one-shot ring would silently profile
    /// only the first five seconds.
    #[test]
    fn ring_wraps_after_draining() {
        let ring = Ring::new();
        for cycle in 0..3_u64 {
            for index in 0..RING_CAPACITY as u64 {
                let mut frame = Frame::default();
                frame.index = cycle * RING_CAPACITY as u64 + index;
                assert!(ring.push(frame));
            }
            for index in 0..RING_CAPACITY as u64 {
                assert_eq!(
                    ring.pop().expect("published frame").index,
                    cycle * RING_CAPACITY as u64 + index
                );
            }
        }
    }

    /// A disabled profile must not read the clock, allocate, or publish. This
    /// is what lets the instrumentation stay in the shipping build.
    #[test]
    fn disabled_profile_publishes_nothing() {
        assert!(!enabled(), "profiling must default to off");
        let before = ring().head.load(Ordering::Relaxed);
        let mut profile = BlockProfile::begin(64, 2);
        profile.enter(Item::Render);
        profile.count(Item::RouteSerial, 64);
        profile.finish(4);
        assert_eq!(ring().head.load(Ordering::Relaxed), before);
    }

    /// Phases are exclusive: entering a second phase closes the first, so the
    /// parts never sum past the whole.
    #[test]
    fn phases_are_exclusive_and_bounded_by_the_block() {
        let mut profile = BlockProfile {
            active: true,
            started: Some(Instant::now()),
            phase: None,
            frame: Frame::default(),
        };
        profile.enter(Item::Configuration);
        std::thread::yield_now();
        profile.enter(Item::Render);
        std::thread::yield_now();
        profile.leave();
        let voices = profile.frame.values[Item::Configuration as usize];
        let oversampling = profile.frame.values[Item::Render as usize];
        assert!(voices > 0 && oversampling > 0, "phases recorded no time at all");

        let started = profile.started.expect("active profile has a start");
        let block = u32::try_from(started.elapsed().as_nanos()).unwrap_or(u32::MAX);
        assert!(
            voices.saturating_add(oversampling) <= block,
            "phases {voices}+{oversampling} exceed the block {block}"
        );
    }

    /// Counted routes accumulate rather than overwrite, because a block can
    /// take the same route in several separate stretches.
    #[test]
    fn counted_routes_accumulate() {
        let mut profile = BlockProfile {
            active: true,
            started: Some(Instant::now()),
            phase: None,
            frame: Frame::default(),
        };
        profile.count(Item::RouteBlockMajor, 32);
        profile.count(Item::RouteBlockMajor, 16);
        assert_eq!(profile.frame.values[Item::RouteBlockMajor as usize], 48);
    }
}
