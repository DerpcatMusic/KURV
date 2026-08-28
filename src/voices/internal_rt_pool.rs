use super::{
    ActiveOscillatorRenderSet, EnvelopeSettings, LEGACY_OSCILLATOR_COUNT, MASTER_HEADROOM,
    POLYPHONY, PolySynth, StructuralOscillatorFrameControl, VaVoice, VoiceSettings,
    wrap_swarm_time,
};
use std::cell::UnsafeCell;
use std::hint::spin_loop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::super::poly_synth::VoiceStructuralRouteFrame;
use crate::filters::{FilterCoefficients, FilterConfig};
use crate::generators::{GeneratorRtGroup, MAX_FILTERS};
use crate::modulators::lfo::VoiceLfoProgram;

const HELPERS: usize = 7;
pub const MAX_JOB_SAMPLES: usize = 512;
const EXACT_WAIT_CAP: Duration = Duration::from_millis(2);
const GENERIC_WAIT_CAP: Duration = Duration::from_millis(5);
const ADAPTIVE_WAIT_CAP: Duration = Duration::from_millis(16);

type StereoBlock = [(f32, f32); MAX_JOB_SAMPLES];

#[derive(Clone, Copy)]
struct TerminalFilterJob<'a> {
    group: &'a GeneratorRtGroup,
    configs: &'a [FilterConfig; MAX_FILTERS],
    coefficients: &'a [FilterCoefficients; MAX_FILTERS],
    voice_modulation: bool,
}

fn boxed_array<T, const N: usize>(mut make: impl FnMut(usize) -> T) -> Box<[T; N]> {
    let mut array = Box::<[T; N]>::new_uninit();
    let pointer = array.as_mut_ptr().cast::<T>();
    for index in 0..N {
        // SAFETY: every element is written exactly once before the box is assumed initialized.
        unsafe {
            pointer.add(index).write(make(index));
        }
    }
    // SAFETY: the loop above initialized every element of the array.
    unsafe { array.assume_init() }
}

pub struct InternalPoolBlock {
    pub samples: StereoBlock,
    pub len: usize,
}

#[repr(align(64))]
struct WorkerSignal {
    done_epoch: AtomicU32,
    participation: AtomicU64,
    fifo: AtomicBool,
    priority_checked: AtomicBool,
}

impl WorkerSignal {
    const fn new() -> Self {
        Self {
            done_epoch: AtomicU32::new(0),
            participation: AtomicU64::new(0),
            fifo: AtomicBool::new(false),
            priority_checked: AtomicBool::new(false),
        }
    }
}

struct Shared {
    epoch: AtomicU32,
    extra_epoch: AtomicU32,
    cancel_epoch: AtomicU32,
    shutdown: AtomicBool,
    active_helpers: AtomicU32,
    shadow: UnsafeCell<Box<[VaVoice; POLYPHONY]>>,
    shadow_ptr: *mut VaVoice,
    voice_count: AtomicUsize,
    next_voice: AtomicUsize,
    voice_ready: [AtomicU32; POLYPHONY],
    chunk_samples: AtomicUsize,
    job_samples: AtomicUsize,
    sample_rate_bits: AtomicU32,
    exact_saw: AtomicBool,
    block_shape: AtomicBool,
    morphing: AtomicBool,
    structural_modulation: AtomicBool,
    voice_structural_modulation: AtomicBool,
    terminal_filter: AtomicBool,
    voice_filter_modulation: AtomicBool,
    settings: UnsafeCell<VoiceSettings>,
    extended: UnsafeCell<Box<ActiveOscillatorRenderSet>>,
    voice_lfo_program: UnsafeCell<Box<VoiceLfoProgram>>,
    voice_structural_routes: UnsafeCell<VoiceStructuralRouteFrame>,
    filter_group: UnsafeCell<GeneratorRtGroup>,
    filter_configs: UnsafeCell<[FilterConfig; MAX_FILTERS]>,
    filter_coefficients: UnsafeCell<[FilterCoefficients; MAX_FILTERS]>,
    clocks: UnsafeCell<[[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
    shapes: UnsafeCell<[[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
    structural_controls: UnsafeCell<Box<[StructuralOscillatorFrameControl; MAX_JOB_SAMPLES]>>,
    _contributions: UnsafeCell<Box<[StereoBlock; POLYPHONY]>>,
    contributions_ptr: *mut StereoBlock,
    workers: [WorkerSignal; HELPERS],
}

// The audio thread publishes immutable job metadata before `epoch` with Release ordering.
// Each atomic claim owns one disjoint voice and contribution row until its ready epoch is
// published. Per-helper done epochs prevent the next job from reusing metadata while an old
// helper is still leaving its claim loop.
// SAFETY: all non-atomic shared fields follow the epoch/claim ownership protocol above.
unsafe impl Sync for Shared {}
// SAFETY: the raw pointers address their owning boxes above, whose allocations remain stable
// until all helper threads have joined and Shared is dropped.
unsafe impl Send for Shared {}

impl Shared {
    fn new() -> Self {
        let mut shadow = boxed_array(|_| VaVoice::default());
        let shadow_ptr = shadow.as_mut_ptr();
        let mut contributions = boxed_array(|_| [(0.0, 0.0); MAX_JOB_SAMPLES]);
        let contributions_ptr = contributions.as_mut_ptr();
        Self {
            epoch: AtomicU32::new(0),
            extra_epoch: AtomicU32::new(0),
            cancel_epoch: AtomicU32::new(0),
            shutdown: AtomicBool::new(false),
            active_helpers: AtomicU32::new(0),
            shadow: UnsafeCell::new(shadow),
            shadow_ptr,
            voice_count: AtomicUsize::new(0),
            next_voice: AtomicUsize::new(0),
            voice_ready: std::array::from_fn(|_| AtomicU32::new(0)),
            chunk_samples: AtomicUsize::new(0),
            job_samples: AtomicUsize::new(0),
            sample_rate_bits: AtomicU32::new(44_100.0_f32.to_bits()),
            exact_saw: AtomicBool::new(true),
            block_shape: AtomicBool::new(true),
            morphing: AtomicBool::new(false),
            structural_modulation: AtomicBool::new(false),
            voice_structural_modulation: AtomicBool::new(false),
            terminal_filter: AtomicBool::new(false),
            voice_filter_modulation: AtomicBool::new(false),
            settings: UnsafeCell::new(VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)),
            extended: UnsafeCell::new(Box::new(ActiveOscillatorRenderSet::default())),
            voice_lfo_program: UnsafeCell::new(Box::new(VoiceLfoProgram::default())),
            voice_structural_routes: UnsafeCell::new(VoiceStructuralRouteFrame::default()),
            filter_group: UnsafeCell::new(GeneratorRtGroup::EMPTY),
            filter_configs: UnsafeCell::new([FilterConfig::default(); MAX_FILTERS]),
            filter_coefficients: UnsafeCell::new([FilterCoefficients::default(); MAX_FILTERS]),
            clocks: UnsafeCell::new([[0.0; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]),
            shapes: UnsafeCell::new([[0.0; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]),
            structural_controls: UnsafeCell::new(boxed_array(|_| {
                StructuralOscillatorFrameControl::NEUTRAL
            })),
            _contributions: UnsafeCell::new(contributions),
            contributions_ptr,
            workers: std::array::from_fn(|_| WorkerSignal::new()),
        }
    }
}

pub struct InternalRtPool {
    shared: Arc<Shared>,
    handles: [Option<JoinHandle<()>>; HELPERS],
    available_mask: u8,
    jobs: u32,
    in_flight: u32,
    in_flight_helpers: u8,
    deadline_fallbacks: u64,
    voice_sample_ns: [u64; 2],
    workload_signature: [u64; 2],
    #[cfg(test)]
    forced_timeouts: u8,
}

impl Default for InternalRtPool {
    fn default() -> Self {
        Self::new()
    }
}

impl InternalRtPool {
    pub fn new() -> Self {
        let shared = Arc::new(Shared::new());
        let mut handles: [Option<JoinHandle<()>>; HELPERS] = std::array::from_fn(|_| None);
        let mut available_mask = 0_u8;
        let helper_count = thread::available_parallelism()
            .map_or(0, |cores| cores.get().saturating_sub(1).min(HELPERS));
        if helper_count != 0 {
            for (worker, handle) in handles.iter_mut().enumerate().take(helper_count) {
                let worker_shared = Arc::clone(&shared);
                let spawn = thread::Builder::new()
                    .name(format!("kurv-rt-helper-{}", worker + 1))
                    .spawn(move || worker_loop(&worker_shared, worker));
                if let Ok(spawned) = spawn {
                    *handle = Some(spawned);
                    available_mask |= 1 << worker;
                }
            }
        }
        if available_mask != 0 {
            let deadline = Instant::now() + Duration::from_millis(100);
            while !shared.workers.iter().enumerate().all(|(index, worker)| {
                available_mask & (1 << index) == 0
                    || worker.priority_checked.load(Ordering::Acquire)
            }) && Instant::now() < deadline
            {
                thread::yield_now();
            }
        }
        #[cfg(target_os = "windows")]
        {
            // A normal-priority helper can be starved by an MMCSS audio callback that waits for
            // it. Keep every successfully elevated helper instead of disabling the whole pool
            // when only one registration fails.
            for (index, worker) in shared.workers.iter().enumerate() {
                if !worker.fifo.load(Ordering::Acquire) {
                    available_mask &= !(1 << index);
                }
            }
        }
        shared
            .active_helpers
            .store(u32::from(available_mask), Ordering::Release);
        Self {
            shared,
            handles,
            available_mask,
            jobs: 0,
            in_flight: 0,
            in_flight_helpers: 0,
            deadline_fallbacks: 0,
            voice_sample_ns: [0; 2],
            workload_signature: [0; 2],
            #[cfg(test)]
            forced_timeouts: 0,
        }
    }

    pub fn render_saw_block<const SAMPLES: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> Option<[(f32, f32); SAMPLES]> {
        let block = self.render_saw_job::<SAMPLES>(synth, settings, envelope, 1)?;
        Some(std::array::from_fn(|frame| block.samples[frame]))
    }

    pub fn render_saw_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
    ) -> Option<InternalPoolBlock> {
        #[cfg(test)]
        if settings.antialiasing == crate::oscillators::Antialiasing::Spectral {
            return None;
        }
        if !synth.exact_saw_banks_eligible(settings) {
            return None;
        }
        self.render_block_job::<CHUNK>(synth, settings, envelope, chunks)
    }

    pub fn render_block_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
    ) -> Option<InternalPoolBlock> {
        self.render_job::<CHUNK>(synth, settings, envelope, chunks, None, None, false, None)
    }

    pub fn render_morph_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        shapes: &[[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> Option<InternalPoolBlock> {
        self.render_job::<CHUNK>(
            synth,
            settings,
            envelope,
            chunks,
            Some(shapes),
            None,
            false,
            None,
        )
    }

    pub fn render_structural_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        controls: &[StructuralOscillatorFrameControl],
    ) -> Option<InternalPoolBlock> {
        self.render_job::<CHUNK>(
            synth,
            settings,
            envelope,
            chunks,
            None,
            Some(controls),
            false,
            None,
        )
    }

    pub fn render_voice_structural_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        controls: &[StructuralOscillatorFrameControl],
    ) -> Option<InternalPoolBlock> {
        self.render_job::<CHUNK>(
            synth,
            settings,
            envelope,
            chunks,
            None,
            Some(controls),
            true,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_terminal_filter_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        group: &GeneratorRtGroup,
        configs: &[FilterConfig; MAX_FILTERS],
        coefficients: &[FilterCoefficients; MAX_FILTERS],
        voice_modulation: bool,
    ) -> Option<InternalPoolBlock> {
        let settings = synth.apply_oscillator_state(settings);
        self.render_job::<CHUNK>(
            synth,
            settings,
            envelope,
            chunks,
            None,
            None,
            false,
            Some(TerminalFilterJob {
                group,
                configs,
                coefficients,
                voice_modulation,
            }),
        )
    }

    fn render_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        shapes: Option<&[[f32; MAX_JOB_SAMPLES]; LEGACY_OSCILLATOR_COUNT]>,
        structural_controls: Option<&[StructuralOscillatorFrameControl]>,
        voice_structural: bool,
        filter_job: Option<TerminalFilterJob<'_>>,
    ) -> Option<InternalPoolBlock> {
        let job_samples = CHUNK.checked_mul(chunks)?;
        let structural = structural_controls.is_some();
        let terminal_filter = filter_job.is_some();
        let voice_filter_modulation = filter_job.is_some_and(|job| job.voice_modulation);
        let filter_signature =
            filter_job.map_or(0, |job| terminal_filter_signature(job.group, job.configs));
        let available_helpers = self.available_mask.count_ones() as usize;
        if self.in_flight != 0 {
            if !self.helpers_quiescent(self.in_flight, self.in_flight_helpers) {
                return None;
            }
            self.in_flight = 0;
            self.in_flight_helpers = 0;
        }
        if self.available_mask == 0
            || !self.helpers_ready()
            || !matches!(CHUNK, 16 | 24 | 32)
            || chunks == 0
            || job_samples > MAX_JOB_SAMPLES
            || !pool_eligible(synth)
            || synth.oscillator_bank.transitioning()
            || shapes.is_some() && !synth.morph_block_eligible(settings)
            || structural
                && (!(if voice_structural {
                    synth.voice_structural_modulation_block_eligible(settings)
                } else {
                    synth.structural_modulation_block_eligible(settings)
                }) || structural_controls.is_none_or(|controls| controls.len() < job_samples))
            || filter_job.is_some_and(|job| {
                !synth.terminal_filter_block_eligible(settings, envelope, job.group)
                    || job.voice_modulation && !synth.voice_filter_modulation_only()
            })
        {
            return None;
        }
        if synth.envelope != envelope {
            synth.envelope = envelope;
            for voice in &mut synth.voices {
                voice.configure(envelope);
            }
        }
        let voice_count = usize::from(synth.active_count);
        let oscillator_bank = synth.oscillator_bank.render();
        let exact_saw = shapes.is_none() && synth.exact_saw_banks_eligible(settings);
        let block_shape = !oscillator_bank.active() && synth.block_shape_banks_eligible(settings);
        let cost_class = usize::from(!exact_saw);
        let signature = workload_signature(
            synth,
            settings,
            oscillator_bank,
            shapes.is_some(),
            structural,
            voice_structural,
            terminal_filter,
            voice_filter_modulation,
            filter_signature,
            block_shape,
        );
        let calibrating = self.workload_signature[cost_class] != signature;
        if calibrating {
            self.workload_signature[cost_class] = signature;
            self.voice_sample_ns[cost_class] = 0;
        }
        let nominal_budget = nominal_wait_budget(job_samples, synth.sample_rate, exact_saw);
        let helper_count = adaptive_helper_count(
            voice_count,
            available_helpers,
            job_samples,
            nominal_budget,
            self.voice_sample_ns[cost_class],
        );
        if helper_count == 0 && !calibrating {
            return None;
        }

        // SAFETY: no prior job remains in flight and workers cannot observe this metadata until
        // the Release publication below.
        let mut clock_ends = [0.0_f64; LEGACY_OSCILLATOR_COUNT];
        // SAFETY: the audio thread has exclusive access until the epoch Release publication.
        unsafe {
            let clocks = &mut *self.shared.clocks.get();
            for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                if settings.oscillator(oscillator).enabled {
                    let (mut time, step) = if oscillator == 0 {
                        (synth.swarm_time, synth.swarm_step)
                    } else {
                        (
                            synth.secondary_swarm_time[oscillator - 1],
                            synth.secondary_swarm_step[oscillator - 1],
                        )
                    };
                    for value in &mut clocks[oscillator][..job_samples] {
                        time = wrap_swarm_time(time + step);
                        *value = time as f32;
                    }
                    clock_ends[oscillator] = time;
                }
            }
        }
        // Workers mutate a fixed shadow copy. A missed deadline can therefore fall back to the
        // untouched live synth without waiting for a lower-priority helper.
        let mut voice_indices = [0_u8; POLYPHONY];
        let mut packed_voice_count = 0_usize;
        // SAFETY: no prior job remains in flight and only the audio thread writes before publish.
        unsafe {
            let shadow = &mut **self.shared.shadow.get();
            for (source_index, source) in synth.voices.iter().enumerate() {
                if source.active() {
                    voice_indices[packed_voice_count] = source_index as u8;
                    prepare_saw_state(
                        &mut shadow[packed_voice_count],
                        source,
                        settings,
                        oscillator_bank,
                    );
                    if voice_structural {
                        shadow[packed_voice_count].modulation = source.modulation;
                    }
                    if let Some(filter_job) = filter_job {
                        shadow[packed_voice_count].copy_terminal_filter_state_from(
                            source,
                            filter_job.group,
                            (!filter_job.voice_modulation).then_some(filter_job.coefficients),
                        );
                    }
                    if voice_filter_modulation {
                        shadow[packed_voice_count].modulation = source.modulation;
                    }
                    packed_voice_count += 1;
                }
            }
        }
        debug_assert_eq!(packed_voice_count, voice_count);
        let mut remaining_helpers = self.available_mask;
        let mut active_helpers = 0_u8;
        for _ in 0..helper_count {
            let helper = remaining_helpers & remaining_helpers.wrapping_neg();
            active_helpers |= helper;
            remaining_helpers &= !helper;
        }
        self.shared
            .active_helpers
            .store(u32::from(active_helpers), Ordering::Relaxed);

        let epoch = self.jobs.wrapping_add(1).max(1);
        if epoch == 1 && self.jobs != 0 {
            self.shared.cancel_epoch.store(0, Ordering::Relaxed);
            self.shared.extra_epoch.store(0, Ordering::Relaxed);
            for ready in &self.shared.voice_ready {
                ready.store(0, Ordering::Relaxed);
            }
            for worker in &self.shared.workers {
                worker.done_epoch.store(0, Ordering::Relaxed);
            }
        }
        self.jobs = epoch;
        self.shared
            .voice_count
            .store(voice_count, Ordering::Relaxed);
        // Reserve one disjoint voice row for every active helper. Without
        // this, the audio thread can claim the whole queue before a helper
        // wakes, making calibration and participation nondeterministic.
        self.shared
            .next_voice
            .store(helper_count, Ordering::Relaxed);
        self.shared.chunk_samples.store(CHUNK, Ordering::Relaxed);
        self.shared
            .job_samples
            .store(job_samples, Ordering::Relaxed);
        self.shared
            .sample_rate_bits
            .store(synth.sample_rate.to_bits(), Ordering::Relaxed);
        self.shared.exact_saw.store(exact_saw, Ordering::Relaxed);
        self.shared
            .block_shape
            .store(block_shape, Ordering::Relaxed);
        // SAFETY: no worker can observe this job before the Release store to epoch.
        unsafe {
            *self.shared.settings.get() = settings;
            let extended = &mut **self.shared.extended.get();
            extended.copy_from(oscillator_bank);
            if let Some(shapes) = shapes {
                *self.shared.shapes.get() = *shapes;
            }
            if let Some(controls) = structural_controls {
                (&mut **self.shared.structural_controls.get())[..job_samples]
                    .copy_from_slice(&controls[..job_samples]);
            }
            if voice_structural {
                let (program, routes) = synth.voice_structural_job_context();
                (&mut **self.shared.voice_lfo_program.get()).copy_from(program);
                *self.shared.voice_structural_routes.get() = routes;
            }
            if let Some(filter_job) = filter_job {
                *self.shared.filter_group.get() = *filter_job.group;
                *self.shared.filter_configs.get() = *filter_job.configs;
                *self.shared.filter_coefficients.get() = *filter_job.coefficients;
                if filter_job.voice_modulation {
                    let (program, routes) = synth.voice_structural_job_context();
                    (&mut **self.shared.voice_lfo_program.get()).copy_from(program);
                    *self.shared.voice_structural_routes.get() = routes;
                }
            }
        }
        self.shared
            .morphing
            .store(shapes.is_some(), Ordering::Relaxed);
        self.shared
            .structural_modulation
            .store(structural, Ordering::Relaxed);
        self.shared
            .voice_structural_modulation
            .store(voice_structural, Ordering::Relaxed);
        self.shared
            .terminal_filter
            .store(terminal_filter, Ordering::Relaxed);
        self.shared
            .voice_filter_modulation
            .store(voice_filter_modulation, Ordering::Relaxed);
        let participants = helper_count + 1;
        let wait_budget = adaptive_wait_budget(
            nominal_budget,
            voice_count,
            job_samples,
            participants,
            self.voice_sample_ns[cost_class],
        );
        // The first job of a new workload also wakes helpers to seed the cost
        // model. Keep that calibration wait bounded, but allow worker wake-up
        // latency to fit inside the normal exact/generic cap.
        let wait_budget = if calibrating {
            wait_budget.max(if exact_saw {
                EXACT_WAIT_CAP
            } else {
                GENERIC_WAIT_CAP
            })
        } else {
            wait_budget
        };
        let job_started = Instant::now();
        let deadline = Some(job_started + wait_budget);
        self.shared.epoch.store(epoch, Ordering::Release);
        if active_helpers != 0 {
            atomic_wait::wake_all(&self.shared.epoch);
            if active_helpers & 0b111_1000 != 0 {
                self.shared.extra_epoch.store(epoch, Ordering::Release);
                atomic_wait::wake_all(&self.shared.extra_epoch);
            }
        }
        self.in_flight = epoch;
        // Workers 0..2 share one futex and workers 3..6 share another. A wake therefore
        // publishes work to the entire corresponding cohort, including inactive helpers that
        // still have to acknowledge this epoch before its metadata may be reused.
        self.in_flight_helpers = (if active_helpers & 0b000_0111 != 0 {
            self.available_mask & 0b000_0111
        } else {
            0
        }) | if active_helpers & 0b111_1000 != 0 {
            self.available_mask & 0b111_1000
        } else {
            0
        };

        #[cfg(test)]
        if self.forced_timeouts != 0 {
            self.forced_timeouts -= 1;
            self.shared.cancel_epoch.store(epoch, Ordering::Release);
            self.deadline_fallbacks += 1;
            return None;
        }

        // SAFETY: each participant atomically claims a unique shadow voice.
        unsafe { process_claims::<CHUNK>(&self.shared, None, deadline) };
        let mut output = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
        // Reduce completed rows in voice order while helpers finish the tail. The ready epoch's
        // Acquire preserves the serial renderer's exact floating-point addition order.
        // SAFETY: a row is only read after its ready epoch is acquired below.
        for index in 0..voice_count {
            let mut spins = 0_u16;
            while self.shared.voice_ready[index].load(Ordering::Acquire) != epoch {
                spins = spins.wrapping_add(1);
                if spins.is_multiple_of(256)
                    && deadline.is_some_and(|deadline| Instant::now() >= deadline)
                {
                    self.observe_job_cost(
                        epoch,
                        job_started.elapsed(),
                        voice_count,
                        participants,
                        job_samples,
                        cost_class,
                    );
                    self.shared.cancel_epoch.store(epoch, Ordering::Release);
                    self.deadline_fallbacks += 1;
                    // Workers only mutate the shadow copy. Leave the job in flight so the next
                    // callback cannot reuse its metadata, and let the shell render the untouched
                    // live synth serially without waiting for a preempted helper.
                    return None;
                }
                spin_loop();
            }
            // SAFETY: this row's ready epoch was acquired above, so its unique writer has
            // finished and no participant can claim it again during this job.
            let voice = unsafe { &*self.shared.contributions_ptr.add(index) };
            for frame in 0..job_samples {
                output[frame].0 += voice[frame].0;
                output[frame].1 += voice[frame].1;
            }
        }
        for sample in &mut output[..job_samples] {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        self.observe_job_cost(
            epoch,
            job_started.elapsed(),
            voice_count,
            participants,
            job_samples,
            cost_class,
        );
        // SAFETY: every voice's ready epoch was acquired above, proving all shadow writes done.
        // Jobs only advance oscillator, jitter, and envelope state.
        // Keep immutable layouts in place instead of copying the full voice.
        unsafe {
            let mut finished = 0_u8;
            for packed_index in 0..voice_count {
                // SAFETY: every voice row published ready before this commit loop.
                let rendered = &*self.shared.shadow_ptr.add(packed_index);
                let live = &mut synth.voices[usize::from(voice_indices[packed_index])];
                let was_active = live.active();
                commit_saw_state(live, rendered, settings, oscillator_bank);
                if voice_structural {
                    live.modulation = rendered.modulation;
                }
                if let Some(filter_job) = filter_job {
                    live.copy_terminal_filter_state_from(
                        rendered,
                        filter_job.group,
                        (!filter_job.voice_modulation).then_some(filter_job.coefficients),
                    );
                }
                if voice_filter_modulation {
                    live.modulation = rendered.modulation;
                }
                finished += u8::from(was_active && !live.active());
            }
            synth.active_count = synth.active_count.saturating_sub(finished);
        }
        if settings.oscillator(0).enabled {
            synth.swarm_time = clock_ends[0];
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if settings.oscillator(secondary + 1).enabled {
                synth.secondary_swarm_time[secondary] = clock_ends[secondary + 1];
            }
        }
        Some(InternalPoolBlock {
            samples: output,
            len: job_samples,
        })
    }

    fn helpers_quiescent(&self, epoch: u32, active_helpers: u8) -> bool {
        self.shared
            .workers
            .iter()
            .enumerate()
            .all(|(index, worker)| {
                active_helpers & (1 << index) == 0
                    || worker.done_epoch.load(Ordering::Acquire) == epoch
            })
    }

    fn helpers_ready(&self) -> bool {
        self.shared
            .workers
            .iter()
            .enumerate()
            .all(|(index, worker)| {
                self.available_mask & (1 << index) == 0
                    || worker.priority_checked.load(Ordering::Acquire)
            })
    }

    fn observe_job_cost(
        &mut self,
        epoch: u32,
        elapsed: Duration,
        voice_count: usize,
        participants: usize,
        job_samples: usize,
        cost_class: usize,
    ) {
        let completed = self.shared.voice_ready[..voice_count]
            .iter()
            .filter(|ready| ready.load(Ordering::Acquire) == epoch)
            .count();
        if completed == 0 || job_samples == 0 {
            return;
        }
        let observed = elapsed
            .as_nanos()
            .saturating_mul(participants as u128)
            .checked_div((completed * job_samples) as u128)
            .unwrap_or(0)
            .min(u128::from(u64::MAX)) as u64;
        let estimate = &mut self.voice_sample_ns[cost_class];
        *estimate = if *estimate == 0 {
            observed
        } else {
            estimate.saturating_mul(3).saturating_add(observed) / 4
        };
    }

    #[cfg(test)]
    fn force_timeout_once(&mut self) {
        self.forced_timeouts = 1;
    }

    pub fn worker_participation(&self) -> [u64; 3] {
        std::array::from_fn(|worker| {
            self.shared.workers[worker]
                .participation
                .load(Ordering::Relaxed)
        })
    }

    pub fn worker_participation_all(&self) -> [u64; HELPERS] {
        std::array::from_fn(|worker| {
            self.shared.workers[worker]
                .participation
                .load(Ordering::Relaxed)
        })
    }

    pub fn fifo_workers(&self) -> [bool; 3] {
        std::array::from_fn(|worker| self.shared.workers[worker].fifo.load(Ordering::Relaxed))
    }

    pub fn fifo_workers_all(&self) -> [bool; HELPERS] {
        std::array::from_fn(|worker| self.shared.workers[worker].fifo.load(Ordering::Relaxed))
    }

    pub const fn deadline_fallbacks(&self) -> u64 {
        self.deadline_fallbacks
    }
}

impl Drop for InternalRtPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.epoch.fetch_add(1, Ordering::Release);
        self.shared.extra_epoch.fetch_add(1, Ordering::Release);
        atomic_wait::wake_all(&self.shared.epoch);
        atomic_wait::wake_all(&self.shared.extra_epoch);
        for handle in self.handles.iter_mut() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

fn pool_eligible(synth: &PolySynth) -> bool {
    // RESYNTH settings point into audio-thread-owned playback plans. Keep those plans and their
    // mutable render state on the audio thread until helper ownership is explicitly designed.
    synth.active_count > 1 && synth.unison_layouts_steady() && !synth.has_active_resynth()
}

fn worker_loop(shared: &Shared, worker: usize) {
    let _denormal_guard = truce_core::denormal::DenormalGuard::new();
    let worker_priority = WorkerPriority::request();
    shared.workers[worker]
        .fifo
        .store(worker_priority.active(), Ordering::Relaxed);
    shared.workers[worker]
        .priority_checked
        .store(true, Ordering::Release);
    let mut seen = 0_u32;
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        let wake_epoch = if worker < 3 {
            &shared.epoch
        } else {
            &shared.extra_epoch
        };
        let epoch = wake_epoch.load(Ordering::Acquire);
        if epoch == seen {
            atomic_wait::wait(wake_epoch, seen);
            continue;
        }
        seen = epoch;
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        let active_helpers = shared.active_helpers.load(Ordering::Acquire);
        if active_helpers & (1 << worker) == 0 {
            shared.workers[worker]
                .done_epoch
                .store(epoch, Ordering::Release);
            continue;
        }
        match shared.chunk_samples.load(Ordering::Relaxed) {
            // SAFETY: each helper atomically claims disjoint voice and contribution rows.
            16 => unsafe { process_claims::<16>(shared, Some(worker), None) },
            // SAFETY: each helper atomically claims disjoint voice and contribution rows.
            24 => unsafe { process_claims::<24>(shared, Some(worker), None) },
            // SAFETY: each helper atomically claims disjoint voice and contribution rows.
            32 => unsafe { process_claims::<32>(shared, Some(worker), None) },
            _ => return,
        }
        shared.workers[worker]
            .done_epoch
            .store(epoch, Ordering::Release);
    }
}

fn nominal_wait_budget(job_samples: usize, sample_rate: f32, exact_saw: bool) -> Duration {
    let audio_duration = job_samples as f64 / f64::from(sample_rate.max(1.0));
    if exact_saw {
        Duration::from_secs_f64(audio_duration * 0.75).min(EXACT_WAIT_CAP)
    } else {
        Duration::from_secs_f64(audio_duration * 0.95).min(GENERIC_WAIT_CAP)
    }
}

fn adaptive_helper_count(
    voice_count: usize,
    available_helpers: usize,
    job_samples: usize,
    budget: Duration,
    voice_sample_ns: u64,
) -> usize {
    let max_helpers = available_helpers.min(voice_count.saturating_sub(1));
    if max_helpers == 0 {
        return 0;
    }
    if voice_sample_ns == 0 {
        return max_helpers;
    }
    let total_ns = u128::from(voice_sample_ns)
        .saturating_mul(job_samples as u128)
        .saturating_mul(voice_count as u128);
    let budget_ns = budget.as_nanos().max(1);
    // Waking and synchronizing helpers has a fixed cost. Keep work on the audio thread unless
    // the measured serial job is large enough to offer substantial deadline headroom.
    if total_ns <= budget_ns.saturating_mul(2) {
        return 0;
    }
    let participants = total_ns.div_ceil(budget_ns);
    if participants > (max_helpers + 1) as u128 {
        max_helpers
    } else {
        participants.max(1).saturating_sub(1) as usize
    }
}

fn adaptive_wait_budget(
    nominal: Duration,
    voice_count: usize,
    job_samples: usize,
    participants: usize,
    voice_sample_ns: u64,
) -> Duration {
    let predicted_ns = u128::from(voice_sample_ns)
        .saturating_mul(job_samples as u128)
        .saturating_mul(voice_count as u128)
        .div_ceil(participants.max(1) as u128)
        .saturating_mul(2)
        .min(ADAPTIVE_WAIT_CAP.as_nanos());
    nominal.max(Duration::from_nanos(predicted_ns as u64))
}

fn workload_signature(
    synth: &PolySynth,
    settings: VoiceSettings,
    oscillator_bank: &ActiveOscillatorRenderSet,
    morphing: bool,
    structural: bool,
    voice_structural: bool,
    terminal_filter: bool,
    voice_filter_modulation: bool,
    filter_signature: u64,
    block_shape: bool,
) -> u64 {
    let mut oscillator_count = 0_u64;
    let mut lane_count = 0_u64;
    let mut warp_count = 0_u64;
    let mut custom_count = 0_u64;
    if let Some(voice) = synth.voices.iter().find(|voice| voice.active()) {
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            let oscillator_settings = settings.oscillator(oscillator);
            if oscillator_settings.enabled {
                oscillator_count += 1;
                warp_count += u64::from(oscillator_settings.phase_warp_active());
                custom_count += u64::from(oscillator_settings.custom_active());
                lane_count += if oscillator == 0 {
                    u64::from(voice.unison.render_voices)
                } else {
                    u64::from(voice.secondary_unison[oscillator - 1].render_voices)
                };
            }
        }
    }
    for entry in oscillator_bank.entries() {
        oscillator_count += 1;
        lane_count += u64::from(entry.current.render_voices);
        warp_count += u64::from(entry.current.phase_warp.active());
        custom_count += u64::from(entry.current.custom_mix > f32::EPSILON);
    }
    let antialiasing = match settings.antialiasing {
        crate::oscillators::Antialiasing::Spline => 0_u64,
        crate::oscillators::Antialiasing::SplineOptimized => 1,
        #[cfg(test)]
        crate::oscillators::Antialiasing::Legacy => 2,
        #[cfg(test)]
        crate::oscillators::Antialiasing::Lagrange => 3,
        #[cfg(test)]
        crate::oscillators::Antialiasing::Spectral => 4,
    };
    let (voice_lfo_sources, voice_routes) = if voice_structural || voice_filter_modulation {
        synth.voice_structural_workload()
    } else {
        (0, 0)
    };
    let base = lane_count
        | (oscillator_count << 16)
        | (u64::from(morphing) << 24)
        | (u64::from(structural) << 25)
        | (u64::from(block_shape) << 26)
        | (warp_count << 27)
        | (custom_count << 33)
        | (antialiasing << 39)
        | (u64::from(voice_structural) << 42)
        | (u64::from(voice_lfo_sources.min(63)) << 43)
        | (u64::from(voice_routes.min(63)) << 49)
        | (u64::from(terminal_filter) << 55)
        | (u64::from(voice_filter_modulation) << 56);
    base ^ filter_signature.rotate_left(17)
}

fn terminal_filter_signature(
    group: &GeneratorRtGroup,
    configs: &[FilterConfig; MAX_FILTERS],
) -> u64 {
    let mut signature = 0xcbf2_9ce4_8422_2325_u64;
    for module in group.terminal_filters().unwrap_or_default() {
        if let crate::generators::GeneratorRtModule::Filter(slot) = *module {
            let config = configs[slot.index()];
            signature ^= slot.index() as u64 | (u64::from(config.mode as u8) << 8);
            signature = signature.wrapping_mul(0x100_0000_01b3);
        }
    }
    signature
}

#[inline]
fn prepare_saw_state(
    target: &mut VaVoice,
    source: &VaVoice,
    settings: VoiceSettings,
    oscillator_bank: &ActiveOscillatorRenderSet,
) {
    debug_assert!(source.unison_transitions_steady());
    target.current_note = source.current_note;
    target.voice_id = source.voice_id;
    target.channel = source.channel;
    target.age = source.age;
    target.frequency_hz = source.frequency_hz;
    target.glide_target_hz = source.glide_target_hz;
    target.glide_multiplier = source.glide_multiplier;
    target.glide_remaining = source.glide_remaining;
    target.pitch_ratio = source.pitch_ratio;
    target.sample_rate = source.sample_rate;
    target.enabled_oscillator_mask = source.enabled_oscillator_mask;
    target.note_seed = source.note_seed;
    target.velocity = source.velocity;
    target.pressure = source.pressure;
    target.timbre = source.timbre;
    target.envelope_level = source.envelope_level;
    target.envelope_start = source.envelope_start;
    target.envelope_progress = source.envelope_progress;
    target.envelope_step = source.envelope_step;
    target.stage = source.stage;
    target.held = source.held;
    target.sustained = source.sustained;
    target.envelope = source.envelope;
    if oscillator_bank.active() {
        target
            .oscillator_bank
            .copy_render_state_from(&source.oscillator_bank, oscillator_bank);
    }

    if settings.oscillator(0).enabled {
        target.oscillators[0] = source.oscillators[0];
        target.unison.copy_render_state_from(&source.unison);
        target.phase_steps = source.phase_steps;
        target.phase_steps_dirty = source.phase_steps_dirty;
        target.swarm_clock = source.swarm_clock;
        target.swarm_clock_offset = source.swarm_clock_offset;
        target.swarm_update_remaining = source.swarm_update_remaining;
        target.swarm_pitch_step = source.swarm_pitch_step;
    }
    for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
        if settings.oscillator(oscillator).enabled {
            let secondary = oscillator - 1;
            target.oscillators[oscillator] = source.oscillators[oscillator];
            target.secondary_unison[secondary]
                .copy_render_state_from(&source.secondary_unison[secondary]);
            target.secondary_phase_steps[secondary] = source.secondary_phase_steps[secondary];
            target.secondary_phase_steps_dirty[secondary] =
                source.secondary_phase_steps_dirty[secondary];
            target.secondary_swarm_clock[secondary] = source.secondary_swarm_clock[secondary];
            target.secondary_swarm_clock_offset[secondary] =
                source.secondary_swarm_clock_offset[secondary];
            target.secondary_swarm_update_remaining[secondary] =
                source.secondary_swarm_update_remaining[secondary];
            target.secondary_swarm_pitch_step[secondary] =
                source.secondary_swarm_pitch_step[secondary];
        }
    }
}

#[inline]
fn commit_saw_state(
    live: &mut VaVoice,
    rendered: &VaVoice,
    settings: VoiceSettings,
    oscillator_bank: &ActiveOscillatorRenderSet,
) {
    if settings.oscillator(0).enabled {
        live.oscillators[0] = rendered.oscillators[0];
        live.phase_steps = rendered.phase_steps;
        live.phase_steps_dirty = rendered.phase_steps_dirty;
        live.swarm_clock = rendered.swarm_clock;
        live.swarm_update_remaining = rendered.swarm_update_remaining;
        live.swarm_pitch_step = rendered.swarm_pitch_step;
    }
    for oscillator in 1..LEGACY_OSCILLATOR_COUNT {
        if settings.oscillator(oscillator).enabled {
            let secondary = oscillator - 1;
            live.oscillators[oscillator] = rendered.oscillators[oscillator];
            live.secondary_phase_steps[secondary] = rendered.secondary_phase_steps[secondary];
            live.secondary_phase_steps_dirty[secondary] =
                rendered.secondary_phase_steps_dirty[secondary];
            live.secondary_swarm_clock[secondary] = rendered.secondary_swarm_clock[secondary];
            live.secondary_swarm_update_remaining[secondary] =
                rendered.secondary_swarm_update_remaining[secondary];
            live.secondary_swarm_pitch_step[secondary] =
                rendered.secondary_swarm_pitch_step[secondary];
        }
    }
    if oscillator_bank.active() {
        live.oscillator_bank
            .copy_render_state_from(&rendered.oscillator_bank, oscillator_bank);
    }
    live.current_note = rendered.current_note;
    live.voice_id = rendered.voice_id;
    live.frequency_hz = rendered.frequency_hz;
    live.glide_target_hz = rendered.glide_target_hz;
    live.glide_multiplier = rendered.glide_multiplier;
    live.glide_remaining = rendered.glide_remaining;
    live.pitch_ratio = rendered.pitch_ratio;
    live.envelope_level = rendered.envelope_level;
    live.envelope_start = rendered.envelope_start;
    live.envelope_progress = rendered.envelope_progress;
    live.envelope_step = rendered.envelope_step;
    live.stage = rendered.stage;
    live.held = rendered.held;
    live.sustained = rendered.sustained;
}

#[cfg(not(target_os = "windows"))]
struct WorkerPriority {
    active: bool,
}

#[cfg(not(target_os = "windows"))]
impl WorkerPriority {
    fn request() -> Self {
        #[cfg(target_os = "linux")]
        let active = {
            let parameter = libc::sched_param { sched_priority: 5 };
            // SAFETY: this only changes the calling helper's scheduler policy. Failure (normally
            // EPERM) leaves SCHED_OTHER unchanged, and thread exit restores the default policy.
            unsafe {
                libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &parameter) == 0
            }
        };
        #[cfg(not(target_os = "linux"))]
        let active = false;
        Self { active }
    }

    const fn active(&self) -> bool {
        self.active
    }
}

#[cfg(target_os = "windows")]
struct WorkerPriority {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl WorkerPriority {
    fn request() -> Self {
        use windows_sys::Win32::System::Threading::AvSetMmThreadCharacteristicsW;

        static MMCSS_REGISTRATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
        const PRO_AUDIO: [u16; 10] = [80, 114, 111, 32, 65, 117, 100, 105, 111, 0];
        let mut task_index = 0_u32;
        let registration = match MMCSS_REGISTRATION.lock() {
            Ok(registration) => registration,
            Err(poisoned) => poisoned.into_inner(),
        };
        // SAFETY: PRO_AUDIO is a static nul-terminated UTF-16 string and task_index remains valid
        // for the call. The returned MMCSS handle is owned by this worker and reverted in Drop.
        let handle =
            unsafe { AvSetMmThreadCharacteristicsW(PRO_AUDIO.as_ptr(), &raw mut task_index) };
        drop(registration);
        if handle.is_null() {
            return Self {
                handle: std::ptr::null_mut(),
            };
        }
        Self { handle }
    }

    fn active(&self) -> bool {
        !self.handle.is_null()
    }
}

#[cfg(target_os = "windows")]
impl Drop for WorkerPriority {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: WorkerPriority is constructed and dropped inside the same worker thread.
            let _ = unsafe {
                windows_sys::Win32::System::Threading::AvRevertMmThreadCharacteristics(self.handle)
            };
        }
    }
}

unsafe fn process_claims<const CHUNK: usize>(
    shared: &Shared,
    worker: Option<usize>,
    deadline: Option<Instant>,
) {
    let voice_count = shared.voice_count.load(Ordering::Relaxed);
    let epoch = shared.epoch.load(Ordering::Relaxed);
    let job_samples = shared.job_samples.load(Ordering::Relaxed);
    // SAFETY: the caller owns this job epoch; each claimed index is written by one worker.
    let voices = shared.shadow_ptr;
    let sample_rate = f32::from_bits(shared.sample_rate_bits.load(Ordering::Relaxed));
    // SAFETY: job metadata is immutable until all workers publish completion.
    let settings = unsafe { *shared.settings.get() };
    // SAFETY: job metadata is immutable until all workers publish completion.
    let extended = unsafe { &**shared.extended.get() };
    let legacy_disabled = settings
        .oscillators
        .iter()
        .all(|oscillator| !oscillator.enabled);
    let settled_bank_config = legacy_disabled
        && extended.active()
        && extended
            .entries()
            .iter()
            .all(|entry| !entry.current.jitter_active());
    let block_shape = shared.block_shape.load(Ordering::Relaxed);
    let morphing = shared.morphing.load(Ordering::Relaxed);
    let structural_modulation = shared.structural_modulation.load(Ordering::Relaxed);
    let voice_structural_modulation = shared.voice_structural_modulation.load(Ordering::Relaxed);
    let terminal_filter = shared.terminal_filter.load(Ordering::Relaxed);
    let voice_filter_modulation = shared.voice_filter_modulation.load(Ordering::Relaxed);
    // SAFETY: job metadata is immutable until all workers publish completion.
    let clocks = unsafe { &*shared.clocks.get() };
    // SAFETY: job metadata is immutable until all workers publish completion.
    let shapes = unsafe { &*shared.shapes.get() };
    // SAFETY: the audio thread publishes the used prefix before the job epoch and does not
    // mutate it again until all participating helpers have published completion.
    let structural_controls = unsafe { &**shared.structural_controls.get() };
    // SAFETY: voice modulation job data is immutable until all workers publish completion.
    let voice_lfo_program = unsafe { &**shared.voice_lfo_program.get() };
    // SAFETY: voice modulation job data is immutable until all workers publish completion.
    let voice_structural_routes = unsafe { &*shared.voice_structural_routes.get() };
    // SAFETY: terminal-filter metadata is immutable until all workers acknowledge this epoch.
    let filter_group = unsafe { &*shared.filter_group.get() };
    // SAFETY: terminal-filter metadata is immutable until all workers acknowledge this epoch.
    let filter_configs = unsafe { &*shared.filter_configs.get() };
    // SAFETY: terminal-filter metadata is immutable until all workers acknowledge this epoch.
    let filter_coefficients = unsafe { &*shared.filter_coefficients.get() };
    // SAFETY: each claimed voice owns a disjoint contribution row for this job epoch.
    let output = shared.contributions_ptr;
    let mut participation = 0_u64;
    let mut reserved_voice = worker.map(|worker| {
        let active_helpers = shared.active_helpers.load(Ordering::Relaxed) as u8;
        let lower_mask = active_helpers & ((1_u8 << worker) - 1);
        lower_mask.count_ones() as usize
    });
    loop {
        if worker.is_some() && shared.cancel_epoch.load(Ordering::Acquire) == epoch {
            return;
        }
        let index = reserved_voice
            .take()
            .unwrap_or_else(|| shared.next_voice.fetch_add(1, Ordering::Relaxed));
        if index >= voice_count {
            break;
        }
        if shared.voice_ready[index].load(Ordering::Acquire) == epoch {
            continue;
        }
        // SAFETY: each bank owns a disjoint shadow voice for the duration of this job.
        let voice = unsafe { &mut *voices.add(index) };
        for offset in (0..job_samples).step_by(CHUNK) {
            if worker.is_some() && shared.cancel_epoch.load(Ordering::Acquire) == epoch {
                return;
            }
            let clocks = std::array::from_fn(|oscillator| {
                std::array::from_fn(|frame| clocks[oscillator][offset + frame])
            });
            let shape_frames = morphing.then(|| {
                std::array::from_fn(|oscillator| {
                    std::array::from_fn(|frame| shapes[oscillator][offset + frame])
                })
            });
            let samples = if terminal_filter {
                voice.render_terminal_filter_voice_job::<CHUNK>(
                    settings,
                    sample_rate,
                    extended,
                    filter_group,
                    filter_configs,
                    filter_coefficients,
                    voice_filter_modulation.then_some(voice_lfo_program),
                    voice_filter_modulation.then_some(voice_structural_routes),
                )
            } else if voice_structural_modulation {
                voice.render_voice_structural_modulation_block::<CHUNK>(
                    settings,
                    sample_rate,
                    extended,
                    &structural_controls[offset..offset + CHUNK],
                    voice_lfo_program,
                    voice_structural_routes,
                )
            } else if structural_modulation {
                voice.render_structural_modulation_block::<CHUNK>(
                    settings,
                    sample_rate,
                    extended,
                    &structural_controls[offset..offset + CHUNK],
                )
            } else if extended.active() {
                voice.render_generic_block_with_static_oscillator_bank::<CHUNK>(
                    settings,
                    sample_rate,
                    clocks,
                    shape_frames.as_ref(),
                    extended,
                    legacy_disabled,
                    settled_bank_config,
                )
            } else if let Some(shape_frames) = shape_frames.as_ref() {
                voice.render_morph_block::<CHUNK>(settings, sample_rate, clocks, shape_frames)
            } else if block_shape {
                voice.render_saw_block::<CHUNK>(settings, sample_rate, clocks)
            } else {
                voice.render_generic_block::<CHUNK>(settings, sample_rate, clocks)
            };
            // SAFETY: each bank owns the matching disjoint contribution row.
            unsafe {
                (&mut *output.add(index))[offset..offset + CHUNK].copy_from_slice(&samples);
            }
        }
        participation += 1;
        shared.voice_ready[index].store(epoch, Ordering::Release);
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
    }
    if let Some(worker) = worker {
        shared.workers[worker]
            .participation
            .fetch_add(participation, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Antialiasing, SwarmMode, UnisonSettings};
    use super::*;
    use crate::generators::MAX_OSCILLATORS;
    use crate::pan_curve::PanShapeSegmentsRt;
    use crate::wave_curve::WaveCurveRt;

    fn synth(factor: u8, swarm: f32, mode: SwarmMode) -> PolySynth {
        let mut synth = PolySynth::default();
        synth.set_sample_rate(48_000.0 * f32::from(factor));
        synth.configure_unison(
            UnisonSettings::new(64, 17.0, 1.0, 0.75, 0.0)
                .with_swarm(swarm, 2.3)
                .with_swarm_mode(mode),
        );
        for note in 48..72 {
            synth.note_on(note, 1.0, 0, None);
        }
        synth
    }

    fn structural_config(enabled: bool) -> crate::voices::OscillatorDspConfig {
        crate::voices::OscillatorDspConfig {
            enabled,
            engine: crate::generators::OscillatorEngineKind::Va,
            resynth_playback: crate::voices::ResynthPlaybackPtr::NONE,
            shape: 2.0,
            pulse_width: 0.5,
            custom_curve: WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            phase_warp_mode: 0,
            phase_warp_amount: 0.0,
            phase_mod_source: 0,
            phase_mod_amount: 0.0,
            modulation_mode: crate::generators::GeneratorModMode::Phase,
            transpose: 0.0,
            cents: 0.0,
            level: 0.5,
            pan: 0.0,
            unison_voices: 64,
            unison_range: 1.0,
            unison_amount: 1.0,
            unison_curve: 0.0,
            unison_jitter: 0.0,
            unison_jitter_mode: 0,
            unison_rate: 0.4,
            unison_weight: 0.0,
            unison_width: 1.0,
            phase_position: 0.0,
            phase_random: 1.0,
            unison_alignment: 0.0,
            unison_alignment_mode: 0,
            unison_pan_curve: 0.0,
            unison_pan_center_x: 0.5,
            unison_pan_segments: (PanShapeSegmentsRt::default(), PanShapeSegmentsRt::default()),
            unison_stereo_x: 1.0,
            unison_stereo_alternate: 0.0,
        }
    }

    #[test]
    fn structural_transitions_bypass_voice_partitioning() {
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::SplineOptimized);
        let envelope = EnvelopeSettings::default();
        let mut synth = synth(1, 0.0, SwarmMode::Wander);
        let mut configs = [structural_config(false); MAX_OSCILLATORS];
        configs[0] = structural_config(true);
        synth.configure_oscillators(configs);
        assert!(synth.oscillator_bank.transitioning());

        let mut pool = InternalRtPool::new();
        assert!(
            pool.render_block_job::<32>(&mut synth, settings, envelope, 1)
                .is_none()
        );
        assert!(
            synth
                .render_block::<32>(settings, envelope)
                .iter()
                .all(|(left, right)| left.is_finite() && right.is_finite())
        );
    }

    #[test]
    fn partitioned_render_waits_for_three_helpers_and_matches_serial_bits() {
        let _denormal_guard = truce_core::denormal::DenormalGuard::new();
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::SplineOptimized);
        let envelope = EnvelopeSettings::default();
        let mut serial = synth(2, 0.0, SwarmMode::Wander);
        let mut partitioned = synth(2, 0.0, SwarmMode::Wander);
        let expected = serial.render_saw_block::<32>(settings, envelope);
        let mut pool = InternalRtPool::new();
        let actual = pool
            .render_saw_block::<32>(&mut partitioned, settings, envelope)
            .expect("the exact held-saw workload is pool eligible");

        assert_eq!(
            actual.map(|(left, right)| (left.to_bits(), right.to_bits())),
            expected.map(|(left, right)| (left.to_bits(), right.to_bits()))
        );
        assert!(pool.worker_participation().iter().all(|count| *count > 0));
    }

    fn assert_factor_mode<const SAMPLES: usize>(factor: u8, swarm: f32, mode: SwarmMode) {
        let _denormal_guard = truce_core::denormal::DenormalGuard::new();
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::Spline.for_factor(factor));
        let envelope = EnvelopeSettings::default();
        let mut serial = synth(factor, swarm, mode);
        let mut partitioned = synth(factor, swarm, mode);
        let mut pool = InternalRtPool::new();
        for _ in 0..128 {
            let expected = serial.render_saw_block::<SAMPLES>(settings, envelope);
            let actual = pool
                .render_saw_block::<SAMPLES>(&mut partitioned, settings, envelope)
                .unwrap_or_else(|| partitioned.render_saw_block::<SAMPLES>(settings, envelope));
            assert_eq!(
                actual.map(|(left, right)| (left.to_bits(), right.to_bits())),
                expected.map(|(left, right)| (left.to_bits(), right.to_bits()))
            );
        }
        assert!(pool.worker_participation().iter().all(|count| *count > 0));
    }

    #[test]
    fn factors_one_through_four_null_for_off_wander_and_jitter() {
        for factor in [1, 2] {
            assert_factor_mode::<32>(factor, 0.0, SwarmMode::Wander);
            assert_factor_mode::<16>(factor, 1.0, SwarmMode::Wander);
            assert_factor_mode::<32>(factor, 1.0, SwarmMode::Jitter);
        }
        assert_factor_mode::<24>(3, 0.0, SwarmMode::Wander);
        assert_factor_mode::<24>(3, 1.0, SwarmMode::Wander);
        assert_factor_mode::<24>(3, 1.0, SwarmMode::Jitter);
        assert_factor_mode::<32>(4, 0.0, SwarmMode::Wander);
        assert_factor_mode::<32>(4, 1.0, SwarmMode::Wander);
        assert_factor_mode::<32>(4, 1.0, SwarmMode::Jitter);
    }

    fn assert_batched<const CHUNK: usize>(swarm: f32, mode: SwarmMode) {
        let _denormal_guard = truce_core::denormal::DenormalGuard::new();
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::SplineOptimized);
        let envelope = EnvelopeSettings::default();
        let chunks = MAX_JOB_SAMPLES / CHUNK;
        let mut serial = synth(2, swarm, mode);
        let mut partitioned = synth(2, swarm, mode);
        let mut expected = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
        for chunk in 0..chunks {
            let rendered = serial.render_saw_block::<CHUNK>(settings, envelope);
            expected[chunk * CHUNK..(chunk + 1) * CHUNK].copy_from_slice(&rendered);
        }
        let mut pool = InternalRtPool::new();
        let actual = pool
            .render_saw_job::<CHUNK>(&mut partitioned, settings, envelope, chunks)
            .expect("coarse 24x64 job must meet the offline deadline");
        assert_eq!(actual.len, MAX_JOB_SAMPLES);
        assert_eq!(
            actual
                .samples
                .map(|(left, right)| (left.to_bits(), right.to_bits())),
            expected.map(|(left, right)| (left.to_bits(), right.to_bits()))
        );
    }

    #[test]
    fn coarse_jobs_null_for_off_wander_and_jitter() {
        assert_batched::<32>(0.0, SwarmMode::Wander);
        assert_batched::<16>(1.0, SwarmMode::Wander);
        assert_batched::<32>(1.0, SwarmMode::Jitter);
    }

    #[test]
    fn transient_timeout_falls_back_exactly_and_recovers() {
        let _denormal_guard = truce_core::denormal::DenormalGuard::new();
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::SplineOptimized);
        let envelope = EnvelopeSettings::default();
        let chunks = MAX_JOB_SAMPLES / 32;
        let mut serial = synth(2, 1.0, SwarmMode::Jitter);
        let mut candidate = synth(2, 1.0, SwarmMode::Jitter);
        let mut pool = InternalRtPool::new();
        pool.force_timeout_once();
        let mut recovered = false;

        for _ in 0..64 {
            let mut expected = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
            for chunk in 0..chunks {
                let block = serial.render_saw_block::<32>(settings, envelope);
                expected[chunk * 32..(chunk + 1) * 32].copy_from_slice(&block);
            }
            let pooled = pool.render_saw_job::<32>(&mut candidate, settings, envelope, chunks);
            recovered |= pooled.is_some();
            let actual = pooled.map_or_else(
                || {
                    let mut output = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
                    for chunk in 0..chunks {
                        let block = candidate.render_saw_block::<32>(settings, envelope);
                        output[chunk * 32..(chunk + 1) * 32].copy_from_slice(&block);
                    }
                    output
                },
                |block| block.samples,
            );
            assert_eq!(
                actual.map(|(left, right)| (left.to_bits(), right.to_bits())),
                expected.map(|(left, right)| (left.to_bits(), right.to_bits()))
            );
        }
        assert_eq!(pool.deadline_fallbacks(), 1);
        assert!(recovered, "pool stayed disabled after one transient miss");
    }

    #[test]
    fn unsupported_and_release_states_fall_back_without_stale_jobs() {
        let _denormal_guard = truce_core::denormal::DenormalGuard::new();
        let saw = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)
            .with_antialiasing(Antialiasing::SplineOptimized);
        let triangle = VoiceSettings { shape: 1.0, ..saw };
        let spectral = VoiceSettings {
            antialiasing: Antialiasing::Spectral,
            ..saw
        };
        let envelope = EnvelopeSettings {
            release: 0.1,
            ..EnvelopeSettings::default()
        };
        let mut synth = synth(2, 0.0, SwarmMode::Wander);
        let mut pool = InternalRtPool::new();
        assert!(
            pool.render_saw_block::<32>(&mut synth, triangle, envelope)
                .is_none()
        );
        assert!(
            pool.render_saw_block::<32>(&mut synth, spectral, envelope)
                .is_none()
        );
        let _ = pool
            .render_saw_block::<32>(&mut synth, saw, envelope)
            .expect("held saw is eligible");
        synth.note_off(48, 0, None);
        assert!(
            pool.render_saw_block::<32>(&mut synth, saw, envelope)
                .is_none()
        );
        synth.reset();
        assert!(
            pool.render_saw_block::<32>(&mut synth, saw, envelope)
                .is_none()
        );
    }

    #[test]
    fn active_resynth_render_never_crosses_helper_boundary() {
        let mut synth = synth(1, 0.0, SwarmMode::Wander);
        let mut configs = [structural_config(false); MAX_OSCILLATORS];
        configs[0] = structural_config(true);
        configs[0].engine = crate::generators::OscillatorEngineKind::Resynth;
        synth.configure_oscillators(configs);
        synth.oscillator_bank.snap_to_targets();

        assert!(synth.has_active_resynth());
        assert!(synth.active_count > 1 && synth.unison_layouts_steady());
        let settings = VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0);
        assert!(synth.block_internal_samples(settings, 1).is_none());
        let mut pool = InternalRtPool::new();
        assert!(
            pool.render_block_job::<32>(&mut synth, settings, EnvelopeSettings::default(), 1)
                .is_none()
        );
        assert!(
            !pool_eligible(&synth),
            "an active RESYNTH render must never publish its pointer-bearing state to helpers"
        );
    }
}
