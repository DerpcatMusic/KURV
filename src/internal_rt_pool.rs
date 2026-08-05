use super::{
    EnvelopeSettings, MASTER_HEADROOM, OSCILLATOR_COUNT, POLYPHONY, PolySynth, VaVoice,
    VoiceSettings, wrap_swarm_time,
};
use std::cell::UnsafeCell;
use std::hint::spin_loop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const HELPERS: usize = 3;
const BANKS: usize = HELPERS + 1;
pub const MAX_JOB_SAMPLES: usize = 512;
const ALL_HELPERS: u8 = (1 << HELPERS) - 1;
const MAX_WAIT_CAP: Duration = Duration::from_millis(2);
const PERMANENT_DISABLE_MISSES: u8 = 8;
const MAX_COOLDOWN_JOBS: u8 = 32;

#[cfg(target_os = "linux")]
fn pool_trace(message: &'static [u8]) {
    // SAFETY: the byte string is static and valid for this diagnostic write.
    unsafe {
        let _ = libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
    }
}

#[cfg(not(target_os = "linux"))]
fn pool_trace(_message: &'static [u8]) {}

type StereoBlock = [(f32, f32); MAX_JOB_SAMPLES];

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
    shutdown: AtomicBool,
    shadow: UnsafeCell<[VaVoice; POLYPHONY]>,
    voice_count: AtomicUsize,
    next_voice: AtomicUsize,
    voice_ready: [AtomicU32; POLYPHONY],
    chunk_samples: AtomicUsize,
    job_samples: AtomicUsize,
    sample_rate_bits: AtomicU32,
    exact_saw: AtomicBool,
    block_shape: AtomicBool,
    morphing: AtomicBool,
    settings: UnsafeCell<VoiceSettings>,
    clocks: UnsafeCell<[[f32; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT]>,
    shapes: UnsafeCell<[[f32; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT]>,
    contributions: UnsafeCell<[StereoBlock; POLYPHONY]>,
    workers: [WorkerSignal; HELPERS],
}

// The audio thread publishes immutable job metadata before `epoch` with Release ordering.
// Each atomic claim owns one disjoint voice and contribution row until its ready epoch is
// published. Per-helper done epochs prevent the next job from reusing metadata while an old
// helper is still leaving its claim loop.
// SAFETY: all non-atomic shared fields follow the epoch/claim ownership protocol above.
unsafe impl Sync for Shared {}

impl Shared {
    fn new() -> Self {
        Self {
            epoch: AtomicU32::new(0),
            shutdown: AtomicBool::new(false),
            shadow: UnsafeCell::new(std::array::from_fn(|_| VaVoice::default())),
            voice_count: AtomicUsize::new(0),
            next_voice: AtomicUsize::new(0),
            voice_ready: std::array::from_fn(|_| AtomicU32::new(0)),
            chunk_samples: AtomicUsize::new(0),
            job_samples: AtomicUsize::new(0),
            sample_rate_bits: AtomicU32::new(44_100.0_f32.to_bits()),
            exact_saw: AtomicBool::new(true),
            block_shape: AtomicBool::new(true),
            morphing: AtomicBool::new(false),
            settings: UnsafeCell::new(VoiceSettings::new(2.0, 440.0, 0.5, 0.0, 0.0, 0.0)),
            clocks: UnsafeCell::new([[0.0; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT]),
            shapes: UnsafeCell::new([[0.0; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT]),
            contributions: UnsafeCell::new([[(0.0, 0.0); MAX_JOB_SAMPLES]; POLYPHONY]),
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
    deadline_fallbacks: u64,
    consecutive_misses: u8,
    cooldown_jobs: u8,
    disabled: bool,
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
        let enough_cores = thread::available_parallelism().is_ok_and(|cores| cores.get() >= BANKS);
        if enough_cores {
            for (worker, handle) in handles.iter_mut().enumerate() {
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
        if available_mask == ALL_HELPERS {
            let deadline = Instant::now() + Duration::from_millis(100);
            while !shared
                .workers
                .iter()
                .all(|worker| worker.priority_checked.load(Ordering::Acquire))
                && Instant::now() < deadline
            {
                thread::yield_now();
            }
        }
        #[cfg(target_os = "windows")]
        if !shared
            .workers
            .iter()
            .all(|worker| worker.fifo.load(Ordering::Acquire))
        {
            // A normal-priority helper can be starved by an MMCSS audio callback that waits for
            // it. Prefer bounded serial rendering when Windows refuses realtime registration.
            available_mask = 0;
        }
        Self {
            shared,
            handles,
            available_mask,
            jobs: 0,
            in_flight: 0,
            deadline_fallbacks: 0,
            consecutive_misses: 0,
            cooldown_jobs: 0,
            disabled: false,
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
        self.render_job::<CHUNK>(synth, settings, envelope, chunks, None)
    }

    pub fn render_morph_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        shapes: &[[f32; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT],
    ) -> Option<InternalPoolBlock> {
        self.render_job::<CHUNK>(synth, settings, envelope, chunks, Some(shapes))
    }

    fn render_job<const CHUNK: usize>(
        &mut self,
        synth: &mut PolySynth,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        chunks: usize,
        shapes: Option<&[[f32; MAX_JOB_SAMPLES]; OSCILLATOR_COUNT]>,
    ) -> Option<InternalPoolBlock> {
        let job_samples = CHUNK.checked_mul(chunks)?;
        if self.in_flight != 0 {
            if !self.helpers_quiescent(self.in_flight) {
                return None;
            }
            self.in_flight = 0;
        }
        if self.disabled
            || self.available_mask != ALL_HELPERS
            || !self.helpers_ready()
            || !matches!(CHUNK, 16 | 24 | 32)
            || chunks == 0
            || job_samples > MAX_JOB_SAMPLES
            || !contiguous_pool_eligible(synth, settings)
            || shapes.is_some() && !synth.morph_block_eligible(settings)
        {
            return None;
        }
        if self.cooldown_jobs != 0 {
            self.cooldown_jobs -= 1;
            return None;
        }
        if synth.envelope != envelope {
            synth.envelope = envelope;
            for voice in &mut synth.voices {
                voice.configure(envelope);
            }
        }

        // SAFETY: no prior job remains in flight and workers cannot observe this metadata until
        // the Release publication below.
        let mut clock_ends = [0.0_f64; OSCILLATOR_COUNT];
        // SAFETY: the audio thread has exclusive access until the epoch Release publication.
        unsafe {
            let clocks = &mut *self.shared.clocks.get();
            for oscillator in 0..OSCILLATOR_COUNT {
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
        let voice_count = usize::from(synth.active_count);
        // SAFETY: no prior job remains in flight and only the audio thread writes before publish.
        unsafe {
            let shadow = &mut *self.shared.shadow.get();
            for (target, source) in shadow[..voice_count]
                .iter_mut()
                .zip(&synth.voices[..voice_count])
            {
                prepare_saw_state(target, source, settings);
            }
        }

        let epoch = self.jobs.wrapping_add(1).max(1);
        self.jobs = epoch;
        self.shared
            .voice_count
            .store(voice_count, Ordering::Relaxed);
        self.shared.next_voice.store(0, Ordering::Relaxed);
        self.shared.chunk_samples.store(CHUNK, Ordering::Relaxed);
        self.shared
            .job_samples
            .store(job_samples, Ordering::Relaxed);
        self.shared
            .sample_rate_bits
            .store(synth.sample_rate.to_bits(), Ordering::Relaxed);
        let exact_saw = shapes.is_none() && synth.exact_saw_banks_eligible(settings);
        self.shared.exact_saw.store(exact_saw, Ordering::Relaxed);
        self.shared.block_shape.store(
            synth.block_shape_banks_eligible(settings),
            Ordering::Relaxed,
        );
        // SAFETY: no worker can observe this job before the Release store to epoch.
        unsafe {
            *self.shared.settings.get() = settings;
            if let Some(shapes) = shapes {
                *self.shared.shapes.get() = *shapes;
            }
        }
        self.shared
            .morphing
            .store(shapes.is_some(), Ordering::Relaxed);
        let wait_budget = wait_budget(job_samples, synth.sample_rate, exact_saw);
        let deadline = Instant::now() + wait_budget;
        self.shared.epoch.store(epoch, Ordering::Release);
        atomic_wait::wake_all(&self.shared.epoch);
        self.in_flight = epoch;

        #[cfg(test)]
        if self.forced_timeouts != 0 {
            self.forced_timeouts -= 1;
            self.record_timeout();
            return None;
        }

        // SAFETY: each participant atomically claims a unique shadow voice.
        unsafe { process_claims::<CHUNK>(&self.shared, None, Some(deadline)) };
        let mut output = [(0.0_f32, 0.0_f32); MAX_JOB_SAMPLES];
        // Reduce completed rows in voice order while helpers finish the tail. The ready epoch's
        // Acquire preserves the serial renderer's exact floating-point addition order.
        // SAFETY: a row is only read after its ready epoch is acquired below.
        let contributions = unsafe { &*self.shared.contributions.get() };
        for (index, voice) in contributions[..voice_count].iter().enumerate() {
            let mut spins = 0_u16;
            while self.shared.voice_ready[index].load(Ordering::Acquire) != epoch {
                spins = spins.wrapping_add(1);
                if spins.is_multiple_of(256) && Instant::now() >= deadline {
                    self.record_timeout();
                    return None;
                }
                spin_loop();
            }
            for frame in 0..job_samples {
                output[frame].0 += voice[frame].0;
                output[frame].1 += voice[frame].1;
            }
        }
        for sample in &mut output[..job_samples] {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        // SAFETY: every voice's ready epoch was acquired above, proving all shadow writes done.
        // Exact-saw jobs only advance oscillator, jitter, and envelope state. Keep the live
        // voice's immutable layouts and spectral caches in place instead of copying all 6.9 KiB.
        unsafe {
            let shadow = &*self.shared.shadow.get();
            for (live, rendered) in synth.voices[..voice_count]
                .iter_mut()
                .zip(&shadow[..voice_count])
            {
                commit_saw_state(live, rendered, settings);
            }
        }
        if settings.oscillator(0).enabled {
            synth.swarm_time = clock_ends[0];
        }
        for secondary in 0..OSCILLATOR_COUNT - 1 {
            if settings.oscillator(secondary + 1).enabled {
                synth.secondary_swarm_time[secondary] = clock_ends[secondary + 1];
            }
        }
        self.consecutive_misses = 0;
        self.cooldown_jobs = 0;
        Some(InternalPoolBlock {
            samples: output,
            len: job_samples,
        })
    }

    fn helpers_quiescent(&self, epoch: u32) -> bool {
        self.shared
            .workers
            .iter()
            .all(|worker| worker.done_epoch.load(Ordering::Acquire) == epoch)
    }

    fn helpers_ready(&self) -> bool {
        self.shared
            .workers
            .iter()
            .all(|worker| worker.priority_checked.load(Ordering::Acquire))
    }

    fn record_timeout(&mut self) {
        self.deadline_fallbacks += 1;
        self.consecutive_misses = self.consecutive_misses.saturating_add(1);
        let shift = self.consecutive_misses.saturating_sub(1).min(5);
        self.cooldown_jobs = (1_u8 << shift).min(MAX_COOLDOWN_JOBS);
        self.disabled = self.consecutive_misses >= PERMANENT_DISABLE_MISSES;
    }

    #[cfg(test)]
    fn force_timeout_once(&mut self) {
        self.forced_timeouts = 1;
    }

    pub fn worker_participation(&self) -> [u64; HELPERS] {
        std::array::from_fn(|worker| {
            self.shared.workers[worker]
                .participation
                .load(Ordering::Relaxed)
        })
    }

    pub fn fifo_workers(&self) -> [bool; HELPERS] {
        std::array::from_fn(|worker| self.shared.workers[worker].fifo.load(Ordering::Relaxed))
    }

    pub const fn deadline_fallbacks(&self) -> u64 {
        self.deadline_fallbacks
    }
}

impl Drop for InternalRtPool {
    fn drop(&mut self) {
        pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-drop-enter\n");
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.epoch.fetch_add(1, Ordering::Release);
        atomic_wait::wake_all(&self.shared.epoch);
        for (worker, handle) in self.handles.iter_mut().enumerate() {
            if let Some(handle) = handle.take() {
                match worker {
                    0 => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-1-enter\n"),
                    1 => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-2-enter\n"),
                    _ => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-3-enter\n"),
                }
                let _ = handle.join();
                match worker {
                    0 => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-1-return\n"),
                    1 => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-2-return\n"),
                    _ => pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-join-3-return\n"),
                }
            }
        }
        pool_trace(b"KURV_DIAG control=lifecycle stage=rt-pool-drop-return\n");
    }
}

fn contiguous_pool_eligible(synth: &PolySynth, settings: VoiceSettings) -> bool {
    let count = usize::from(synth.active_count);
    settings.antialiasing != super::Antialiasing::Spectral
        && count >= BANKS
        && synth.oscillator_mix_steady()
        && synth.voices[..count]
            .iter()
            .all(|voice| voice.active() && voice.held && !voice.is_gliding())
        && synth.voices[count..].iter().all(|voice| !voice.active())
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
        let epoch = shared.epoch.load(Ordering::Acquire);
        if epoch == seen {
            atomic_wait::wait(&shared.epoch, seen);
            continue;
        }
        seen = epoch;
        if shared.shutdown.load(Ordering::Acquire) {
            return;
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

fn wait_budget(job_samples: usize, sample_rate: f32, exact_saw: bool) -> Duration {
    let audio_duration = job_samples as f64 / f64::from(sample_rate.max(1.0));
    if exact_saw {
        Duration::from_secs_f64(audio_duration * 0.75).min(MAX_WAIT_CAP)
    } else {
        Duration::from_secs_f64(audio_duration * 0.85).min(Duration::from_millis(4))
    }
}

#[inline]
fn prepare_saw_state(target: &mut VaVoice, source: &VaVoice, settings: VoiceSettings) {
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
    target.output_continuity = source.output_continuity;

    if settings.oscillator(0).enabled {
        target.oscillators[0] = source.oscillators[0];
        target.unison.clone_from(&source.unison);
        target.phase_steps = source.phase_steps;
        target.phase_steps_dirty = source.phase_steps_dirty;
        target.swarm_clock = source.swarm_clock;
        target.swarm_update_remaining = source.swarm_update_remaining;
        target.swarm_pitch_step = source.swarm_pitch_step;
    }
    for oscillator in 1..OSCILLATOR_COUNT {
        if settings.oscillator(oscillator).enabled {
            let secondary = oscillator - 1;
            target.oscillators[oscillator] = source.oscillators[oscillator];
            target.secondary_unison[secondary].clone_from(&source.secondary_unison[secondary]);
            target.secondary_phase_steps[secondary] = source.secondary_phase_steps[secondary];
            target.secondary_phase_steps_dirty[secondary] =
                source.secondary_phase_steps_dirty[secondary];
            target.secondary_swarm_clock[secondary] = source.secondary_swarm_clock[secondary];
            target.secondary_swarm_update_remaining[secondary] =
                source.secondary_swarm_update_remaining[secondary];
            target.secondary_swarm_pitch_step[secondary] =
                source.secondary_swarm_pitch_step[secondary];
        }
    }
}

#[inline]
fn commit_saw_state(live: &mut VaVoice, rendered: &VaVoice, settings: VoiceSettings) {
    if settings.oscillator(0).enabled {
        live.oscillators[0] = rendered.oscillators[0];
        live.phase_steps = rendered.phase_steps;
        live.phase_steps_dirty = rendered.phase_steps_dirty;
        live.swarm_clock = rendered.swarm_clock;
        live.swarm_update_remaining = rendered.swarm_update_remaining;
        live.swarm_pitch_step = rendered.swarm_pitch_step;
    }
    for oscillator in 1..OSCILLATOR_COUNT {
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
    live.current_note = rendered.current_note;
    live.voice_id = rendered.voice_id;
    live.envelope_level = rendered.envelope_level;
    live.envelope_start = rendered.envelope_start;
    live.envelope_progress = rendered.envelope_progress;
    live.envelope_step = rendered.envelope_step;
    live.stage = rendered.stage;
    live.held = rendered.held;
    live.sustained = rendered.sustained;
    live.output_continuity = rendered.output_continuity;
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
    let voices = shared.shadow.get().cast::<VaVoice>();
    let sample_rate = f32::from_bits(shared.sample_rate_bits.load(Ordering::Relaxed));
    // SAFETY: job metadata is immutable until all workers publish completion.
    let settings = unsafe { *shared.settings.get() };
    let block_shape = shared.block_shape.load(Ordering::Relaxed);
    let morphing = shared.morphing.load(Ordering::Relaxed);
    // SAFETY: job metadata is immutable until all workers publish completion.
    let clocks = unsafe { &*shared.clocks.get() };
    // SAFETY: job metadata is immutable until all workers publish completion.
    let shapes = unsafe { &*shared.shapes.get() };
    let output = shared.contributions.get().cast::<StereoBlock>();
    let mut participation = 0_u64;
    loop {
        let index = shared.next_voice.fetch_add(1, Ordering::Relaxed);
        if index >= voice_count {
            break;
        }
        // SAFETY: each bank owns a disjoint shadow voice for the duration of this job.
        let voice = unsafe { &mut *voices.add(index) };
        for offset in (0..job_samples).step_by(CHUNK) {
            let clocks = std::array::from_fn(|oscillator| {
                std::array::from_fn(|frame| clocks[oscillator][offset + frame])
            });
            let samples = if morphing {
                let shapes = std::array::from_fn(|oscillator| {
                    std::array::from_fn(|frame| shapes[oscillator][offset + frame])
                });
                voice.render_morph_block::<CHUNK>(settings, sample_rate, clocks, &shapes)
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
}
