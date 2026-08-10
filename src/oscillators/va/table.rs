//! Fixed-capacity morphable virtual-analog curve table.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::wave_curve::{WAVE_CURVE_RT_VALUES, WaveCurveData, WaveCurveRt};

/// Maximum custom frames in one oscillator's virtual-analog table.
pub const MAX_VA_TABLE_FRAMES: usize = 16;

/// Editor/state-thread frames. An empty table falls back to the legacy custom
/// curve, so older projects retain their exact sound.
#[derive(Clone, Debug, PartialEq, State)]
pub struct VaTableData {
    pub frames: Vec<WaveCurveData>,
}

impl Default for VaTableData {
    fn default() -> Self {
        Self { frames: Vec::new() }
    }
}

impl VaTableData {
    /// Compile editable frames into fixed realtime storage.
    #[must_use]
    pub fn compile_rt(&self) -> VaTableRt {
        let mut table = VaTableRt::default();
        let count = self.frames.len().min(MAX_VA_TABLE_FRAMES);
        table.count = count as u8;
        for (target, source) in table.frames[..count].iter_mut().zip(&self.frames) {
            *target = source.compile_rt();
        }
        table
    }
}

/// Precompiled table stored outside the audio callback's stack.
#[derive(Clone, Debug, PartialEq)]
pub struct VaTableRt {
    frames: [WaveCurveRt; MAX_VA_TABLE_FRAMES],
    count: u8,
}

impl Default for VaTableRt {
    fn default() -> Self {
        Self {
            frames: [WaveCurveRt::zero(); MAX_VA_TABLE_FRAMES],
            count: 0,
        }
    }
}

impl VaTableRt {
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.count as usize
    }

    /// Maps the existing Custom Shape control across the procedural source,
    /// legacy custom curve, and every additional table frame.
    #[must_use]
    pub fn select(&self, base: WaveCurveRt, position: f32) -> VaTableSelectionRt {
        if self.count == 0 {
            return VaTableSelectionRt {
                curve: base,
                mix: position.clamp(0.0, 1.0),
            };
        }
        let custom_frames = self.frame_count();
        let scaled = position.clamp(0.0, 1.0) * custom_frames as f32;
        if scaled <= 1.0 {
            return VaTableSelectionRt {
                curve: self.frames[0],
                mix: scaled.min(1.0),
            };
        }

        let frame_position = scaled - 1.0;
        let first = (frame_position.floor() as usize).min(self.frame_count() - 1);
        let second = (first + 1).min(self.frame_count() - 1);
        let first_curve = self.frames[first];
        let second_curve = self.frames[second];
        VaTableSelectionRt {
            curve: WaveCurveRt::interpolate(
                first_curve,
                second_curve,
                frame_position - first as f32,
            ),
            mix: 1.0,
        }
    }
}

struct AtomicVaTable {
    generation: AtomicU32,
    count: AtomicU8,
    words: Box<[AtomicU32]>,
}

impl AtomicVaTable {
    fn new(table: &VaTableRt) -> Self {
        let result = Self {
            generation: AtomicU32::new(0),
            count: AtomicU8::new(0),
            words: (0..MAX_VA_TABLE_FRAMES * WAVE_CURVE_RT_VALUES)
                .map(|_| AtomicU32::new(0))
                .collect(),
        };
        result.store(table);
        result
    }

    fn store(&self, table: &VaTableRt) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.count.store(table.count, Ordering::Relaxed);
        if table.frame_count() == MAX_VA_TABLE_FRAMES {
            self.store_full(table);
        } else {
            self.store_active(table);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    #[inline(never)]
    fn store_full(&self, table: &VaTableRt) {
        for (frame, curve) in table.frames.iter().enumerate() {
            let base = frame * WAVE_CURVE_RT_VALUES;
            for (target, value) in self.words[base..base + WAVE_CURVE_RT_VALUES]
                .iter()
                .zip(curve.coefficients())
            {
                target.store(value.to_bits(), Ordering::Relaxed);
            }
        }
    }

    #[inline(never)]
    fn store_active(&self, table: &VaTableRt) {
        let mut frame = 0;
        while frame < table.frame_count() {
            let base = frame * WAVE_CURVE_RT_VALUES;
            let coefficients = table.frames[frame].coefficients();
            for coefficient in 0..WAVE_CURVE_RT_VALUES {
                self.words[base + coefficient]
                    .store(coefficients[coefficient].to_bits(), Ordering::Relaxed);
            }
            frame += 1;
        }
    }

    fn store_frame(&self, index: usize, curve: WaveCurveRt) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let base = index * WAVE_CURVE_RT_VALUES;
        for (target, value) in self.words[base..base + WAVE_CURVE_RT_VALUES]
            .iter()
            .zip(curve.coefficients())
        {
            target.store(value.to_bits(), Ordering::Relaxed);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn try_load_after(&self, observed_generation: u32) -> Option<(u32, VaTableRt)> {
        let before = self.generation.load(Ordering::Acquire);
        if before == observed_generation || before & 1 != 0 {
            return None;
        }
        let count = self
            .count
            .load(Ordering::Relaxed)
            .min(MAX_VA_TABLE_FRAMES as u8);
        let frames = std::array::from_fn(|frame| {
            let base = frame * WAVE_CURVE_RT_VALUES;
            WaveCurveRt::from_coefficients(std::array::from_fn(|coefficient| {
                f32::from_bits(self.words[base + coefficient].load(Ordering::Relaxed))
            }))
        });
        let table = VaTableRt { frames, count };
        std::sync::atomic::fence(Ordering::Acquire);
        (self.generation.load(Ordering::Acquire) == before).then_some((before, table))
    }
}

/// Persisted editor table with generation-gated lock-free publication.
pub struct VaTableState {
    data: RwLock<VaTableData>,
    rt: AtomicVaTable,
}

impl VaTableState {
    #[must_use]
    pub fn new() -> Self {
        let data = VaTableData::default();
        let rt = AtomicVaTable::new(&data.compile_rt());
        Self {
            data: RwLock::new(data),
            rt,
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> VaTableData {
        self.data
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub(crate) fn frame_snapshot(&self, index: usize) -> Option<WaveCurveData> {
        self.data
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .frames
            .get(index)
            .cloned()
    }

    pub fn replace(&self, data: VaTableData) {
        let data = VaTableData {
            frames: data
                .frames
                .into_iter()
                .take(MAX_VA_TABLE_FRAMES)
                .map(WaveCurveData::sanitized)
                .collect(),
        };
        let rt = data.compile_rt();
        let mut stored = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *stored = data;
        self.rt.store(&rt);
    }

    pub fn edit<R>(&self, edit: impl FnOnce(&mut VaTableData) -> R) -> R {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let result = edit(&mut data);
        data.frames.truncate(MAX_VA_TABLE_FRAMES);
        for frame in &mut data.frames {
            *frame = std::mem::take(frame).sanitized();
        }
        let rt = data.compile_rt();
        self.rt.store(&rt);
        drop(data);
        result
    }

    /// Duplicates the selected frame and materializes an empty table from its
    /// fallback curve on first use. Returns the new selected frame index.
    pub fn duplicate_after(&self, index: usize, fallback: WaveCurveData) -> Option<usize> {
        self.edit(|data| {
            if data.frames.is_empty() {
                data.frames.push(fallback.clone());
                data.frames.push(fallback);
                return Some(1);
            }
            if data.frames.len() >= MAX_VA_TABLE_FRAMES || index >= data.frames.len() {
                return None;
            }
            let inserted = index + 1;
            data.frames.insert(inserted, data.frames[index].clone());
            Some(inserted)
        })
    }

    pub fn materialize(&self, fallback: WaveCurveData) -> bool {
        self.edit(|data| {
            if !data.frames.is_empty() {
                return false;
            }
            data.frames.push(fallback);
            true
        })
    }

    pub fn replace_frame(&self, index: usize, frame: WaveCurveData) -> bool {
        self.edit(|data| {
            let Some(target) = data.frames.get_mut(index) else {
                return false;
            };
            *target = frame;
            true
        })
    }

    pub fn edit_frame<R>(
        &self,
        index: usize,
        edit: impl FnOnce(&mut WaveCurveData) -> R,
    ) -> Option<R> {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let frame = data.frames.get_mut(index)?;
        let result = edit(frame);
        *frame = std::mem::take(frame).sanitized();
        let rt = frame.compile_rt();
        self.rt.store_frame(index, rt);
        Some(result)
    }

    pub fn remove_frame(&self, index: usize) -> bool {
        self.edit(|data| {
            if index >= data.frames.len() {
                return false;
            }
            data.frames.remove(index);
            true
        })
    }

    /// Copy a new table snapshot only when its published generation changed.
    pub fn try_table_rt(&self, observed_generation: u32) -> Option<(u32, VaTableRt)> {
        self.rt.try_load_after(observed_generation)
    }
}

impl Default for VaTableState {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistField for VaTableState {
    fn persist_write(&self, buf: &mut Vec<u8>) {
        self.snapshot().write_field(buf);
    }

    fn persist_read(&self, cursor: &mut StateCursor) {
        if let Some(data) = VaTableData::read_field(cursor) {
            self.replace(data);
        }
    }
}

/// Compiled curve and procedural-to-custom mix consumed by current render
/// kernels. Coefficient interpolation is exact for the curve representation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaTableSelectionRt {
    pub curve: WaveCurveRt,
    pub mix: f32,
}
