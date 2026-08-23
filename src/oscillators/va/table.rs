//! Fixed-capacity morphable virtual-analog curve table.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

use truce::State;
use truce_core::custom_state::{PersistField, StateCursor, StateField};

use crate::wave_curve::{WAVE_CURVE_RT_VALUES, WaveCurveData, WaveCurveRt};

/// Maximum custom frames in one oscillator's virtual-analog table.
pub const MAX_VA_TABLE_FRAMES: usize = 16;
pub const VA_KEYFRAME_EPSILON: f32 = 0.001;
const CANONICAL_WAVE_POSITIONS: [f32; 4] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

fn collides_with_canonical(position: f32) -> bool {
    CANONICAL_WAVE_POSITIONS
        .into_iter()
        .any(|canonical| (canonical - position).abs() <= VA_KEYFRAME_EPSILON)
}

/// Editor/state-thread frames. An empty table falls back to the legacy custom
/// curve, so older projects retain their exact sound.
#[derive(Clone, Debug, PartialEq, State)]
pub struct VaTableData {
    pub frames: Vec<WaveCurveData>,
    /// Normalized WAVE-axis positions for custom keyframes. A nonempty table
    /// with one position per frame uses positioned keyframes; an empty vector
    /// preserves the legacy global Custom Shape table mapping.
    pub positions: Vec<f32>,
}

impl Default for VaTableData {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            positions: Vec::new(),
        }
    }
}

impl VaTableData {
    #[must_use]
    pub fn is_positioned(&self) -> bool {
        !self.frames.is_empty() && self.positions.len() == self.frames.len()
    }

    #[must_use]
    pub fn frame_index_at_position(&self, position: f32) -> Option<usize> {
        self.is_positioned().then_some(())?;
        let position = position.is_finite().then(|| position.clamp(0.0, 1.0))?;
        self.positions
            .iter()
            .position(|candidate| (*candidate - position).abs() <= VA_KEYFRAME_EPSILON)
    }

    #[must_use]
    pub fn nearest_positioned_frame(&self, position: f32) -> Option<usize> {
        self.is_positioned().then_some(())?;
        self.positions
            .iter()
            .enumerate()
            .min_by(|left, right| {
                (left.1 - position)
                    .abs()
                    .total_cmp(&(right.1 - position).abs())
            })
            .map(|(index, _)| index)
    }

    pub(crate) fn sanitized(self) -> Self {
        let positioned = self.is_positioned();
        if !positioned {
            return Self {
                frames: self
                    .frames
                    .into_iter()
                    .take(MAX_VA_TABLE_FRAMES)
                    .map(WaveCurveData::sanitized)
                    .collect(),
                positions: Vec::new(),
            };
        }
        let mut pairs = self
            .positions
            .into_iter()
            .zip(self.frames)
            .filter(|(position, _)| position.is_finite())
            .map(|(position, frame)| (position.clamp(0.0, 1.0), frame.sanitized()))
            .filter(|(position, _)| !collides_with_canonical(*position))
            .collect::<Vec<_>>();
        pairs.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut frames = Vec::with_capacity(pairs.len().min(MAX_VA_TABLE_FRAMES));
        let mut positions = Vec::with_capacity(pairs.len().min(MAX_VA_TABLE_FRAMES));
        for (position, frame) in pairs {
            if positions
                .last()
                .is_some_and(|previous: &f32| (position - *previous).abs() <= 0.000_1)
            {
                if let Some(last) = frames.last_mut() {
                    *last = frame;
                }
                continue;
            }
            if frames.len() == MAX_VA_TABLE_FRAMES {
                break;
            }
            positions.push(position);
            frames.push(frame);
        }
        Self { frames, positions }
    }

    /// Compile editable frames into fixed realtime storage.
    #[must_use]
    pub fn compile_rt(&self) -> VaTableRt {
        let mut table = VaTableRt::default();
        let count = self.frames.len().min(MAX_VA_TABLE_FRAMES);
        table.count = count as u8;
        table.positioned = self.is_positioned();
        for (index, (target, source)) in table.frames[..count]
            .iter_mut()
            .zip(&self.frames)
            .enumerate()
        {
            *target = source.compile_rt();
            if table.positioned {
                table.positions[index] = self.positions[index].clamp(0.0, 1.0);
            }
        }
        table
    }
}

/// Precompiled table stored outside the audio callback's stack.
#[derive(Clone, Debug, PartialEq)]
pub struct VaTableRt {
    frames: [WaveCurveRt; MAX_VA_TABLE_FRAMES],
    positions: [f32; MAX_VA_TABLE_FRAMES],
    count: u8,
    positioned: bool,
}

impl Default for VaTableRt {
    fn default() -> Self {
        Self {
            frames: [WaveCurveRt::zero(); MAX_VA_TABLE_FRAMES],
            positions: [0.0; MAX_VA_TABLE_FRAMES],
            count: 0,
            positioned: false,
        }
    }
}

impl VaTableRt {
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.count as usize
    }

    #[must_use]
    pub const fn is_positioned(&self) -> bool {
        self.positioned
    }

    /// Selects either the legacy global custom table or positioned custom
    /// keyframes on top of the four canonical WAVE anchors.
    #[must_use]
    pub fn select(
        &self,
        base: WaveCurveRt,
        legacy_position: f32,
        wave_position: f32,
    ) -> VaTableSelectionRt {
        let legacy_position = if legacy_position.is_finite() {
            legacy_position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let wave_position = if wave_position.is_finite() {
            wave_position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if !self.positioned {
            return self.select_legacy(base, legacy_position, wave_position);
        }
        self.select_positioned(base, wave_position)
    }

    fn select_legacy(
        &self,
        base: WaveCurveRt,
        position: f32,
        wave_position: f32,
    ) -> VaTableSelectionRt {
        if self.count == 0 {
            return VaTableSelectionRt {
                shape: wave_position.clamp(0.0, 1.0) * 3.0,
                curve: base,
                mix: position.clamp(0.0, 1.0),
            };
        }
        let custom_frames = self.frame_count();
        let scaled = position.clamp(0.0, 1.0) * custom_frames as f32;
        if scaled <= 1.0 {
            return VaTableSelectionRt {
                shape: wave_position.clamp(0.0, 1.0) * 3.0,
                curve: self.frames[0],
                mix: scaled.min(1.0),
            };
        }

        let frame_position = scaled - 1.0;
        let first = (frame_position.floor() as usize).min(self.frame_count() - 1);
        let second = (first + 1).min(self.frame_count() - 1);
        VaTableSelectionRt {
            shape: wave_position.clamp(0.0, 1.0) * 3.0,
            curve: WaveCurveRt::interpolate(
                self.frames[first],
                self.frames[second],
                frame_position - first as f32,
            ),
            mix: 1.0,
        }
    }

    fn select_positioned(&self, base: WaveCurveRt, position: f32) -> VaTableSelectionRt {
        #[derive(Clone, Copy)]
        struct Anchor {
            position: f32,
            custom: Option<usize>,
        }

        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut left = Anchor {
            position: 0.0,
            custom: None,
        };
        let mut right = Anchor {
            position: 1.0,
            custom: None,
        };
        for canonical in CANONICAL_WAVE_POSITIONS {
            if canonical <= position && canonical >= left.position {
                left = Anchor {
                    position: canonical,
                    custom: None,
                };
            }
            if canonical >= position && canonical <= right.position {
                right = Anchor {
                    position: canonical,
                    custom: None,
                };
            }
        }
        for index in 0..self.frame_count() {
            let custom_position = self.positions[index];
            if custom_position <= position && custom_position >= left.position {
                left = Anchor {
                    position: custom_position,
                    custom: Some(index),
                };
            }
            if custom_position >= position && custom_position <= right.position {
                right = Anchor {
                    position: custom_position,
                    custom: Some(index),
                };
            }
        }
        if (right.position - left.position).abs() <= f32::EPSILON {
            return left.custom.or(right.custom).map_or(
                VaTableSelectionRt {
                    shape: position * 3.0,
                    curve: base,
                    mix: 0.0,
                },
                |index| VaTableSelectionRt {
                    shape: position * 3.0,
                    curve: self.frames[index],
                    mix: 1.0,
                },
            );
        }
        let t = ((position - left.position) / (right.position - left.position)).clamp(0.0, 1.0);
        match (left.custom, right.custom) {
            (Some(first), Some(second)) => VaTableSelectionRt {
                shape: position * 3.0,
                curve: WaveCurveRt::interpolate(self.frames[first], self.frames[second], t),
                mix: 1.0,
            },
            (Some(index), None) => VaTableSelectionRt {
                shape: right.position * 3.0,
                curve: self.frames[index],
                mix: 1.0 - t,
            },
            (None, Some(index)) => VaTableSelectionRt {
                shape: left.position * 3.0,
                curve: self.frames[index],
                mix: t,
            },
            (None, None) => VaTableSelectionRt {
                shape: position * 3.0,
                curve: base,
                mix: 0.0,
            },
        }
    }
}

/// Nearest editable frame for a Custom Shape position.
#[must_use]
pub fn nearest_frame_index(position: f32, frame_count: usize) -> usize {
    if frame_count == 0 {
        return 0;
    }
    let scaled = position.clamp(0.0, 1.0) * frame_count as f32;
    (scaled.round() as usize).clamp(1, frame_count) - 1
}

/// Custom Shape value that fully selects `index` in an `frame_count` table.
#[must_use]
pub fn position_for_frame(index: usize, frame_count: usize) -> f32 {
    if frame_count == 0 {
        return 0.0;
    }
    (index.saturating_add(1) as f32 / frame_count as f32).clamp(0.0, 1.0)
}

struct AtomicVaTable {
    generation: AtomicU32,
    count: AtomicU8,
    positioned: AtomicU8,
    positions: Box<[AtomicU32]>,
    words: Box<[AtomicU32]>,
}

impl AtomicVaTable {
    fn new(table: &VaTableRt) -> Self {
        let result = Self {
            generation: AtomicU32::new(0),
            count: AtomicU8::new(0),
            positioned: AtomicU8::new(0),
            positions: (0..MAX_VA_TABLE_FRAMES)
                .map(|_| AtomicU32::new(0))
                .collect(),
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
        self.positioned
            .store(u8::from(table.positioned), Ordering::Relaxed);
        for (target, position) in self.positions.iter().zip(table.positions) {
            target.store(position.to_bits(), Ordering::Relaxed);
        }
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
        let positioned = self.positioned.load(Ordering::Relaxed) != 0;
        let positions = std::array::from_fn(|index| {
            f32::from_bits(self.positions[index].load(Ordering::Relaxed))
        });
        let frames = std::array::from_fn(|frame| {
            let base = frame * WAVE_CURVE_RT_VALUES;
            WaveCurveRt::from_coefficients(std::array::from_fn(|coefficient| {
                f32::from_bits(self.words[base + coefficient].load(Ordering::Relaxed))
            }))
        });
        let table = VaTableRt {
            frames,
            positions,
            count,
            positioned,
        };
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

    #[must_use]
    pub(crate) fn history_generation(&self) -> u32 {
        self.rt.generation.load(Ordering::Acquire)
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
        let data = data.sanitized();
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
        *data = std::mem::take(&mut *data).sanitized();
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
            if data.is_positioned() {
                let current = data.positions[index];
                let next_custom = data.positions.get(index + 1).copied().unwrap_or(1.0);
                let previous_custom = index
                    .checked_sub(1)
                    .and_then(|previous| data.positions.get(previous).copied())
                    .unwrap_or(0.0);
                let next_canonical = CANONICAL_WAVE_POSITIONS
                    .into_iter()
                    .filter(|position| *position > current)
                    .min_by(f32::total_cmp)
                    .unwrap_or(1.0);
                let previous_canonical = CANONICAL_WAVE_POSITIONS
                    .into_iter()
                    .filter(|position| *position < current)
                    .max_by(f32::total_cmp)
                    .unwrap_or(0.0);
                let next = next_custom.min(next_canonical);
                let previous = previous_custom.max(previous_canonical);
                let (inserted, position) = if next - current > VA_KEYFRAME_EPSILON * 2.0 {
                    (index + 1, (current + next) * 0.5)
                } else if current - previous > VA_KEYFRAME_EPSILON * 2.0 {
                    (index, (previous + current) * 0.5)
                } else {
                    return None;
                };
                let frame = data.frames[index].clone();
                data.frames.insert(inserted, frame);
                data.positions.insert(inserted, position);
                return Some(inserted);
            }
            let inserted = index + 1;
            data.frames.insert(inserted, data.frames[index].clone());
            Some(inserted)
        })
    }

    pub fn insert_frame(&self, index: usize, frame: WaveCurveData) -> Option<usize> {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if data.frames.len() >= MAX_VA_TABLE_FRAMES || data.is_positioned() {
            return None;
        }
        let index = index.min(data.frames.len());
        data.frames.insert(index, frame.sanitized());
        let rt = data.compile_rt();
        self.rt.store(&rt);
        Some(index)
    }

    pub fn insert_positioned_frame(&self, position: f32, frame: WaveCurveData) -> Option<usize> {
        let mut data = self
            .data
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if data.frames.len() >= MAX_VA_TABLE_FRAMES
            || (!data.frames.is_empty() && !data.is_positioned())
        {
            return None;
        }
        if !position.is_finite() {
            return None;
        }
        let position = position.clamp(0.0, 1.0);
        if collides_with_canonical(position) {
            return None;
        }
        if let Some(index) = data.frame_index_at_position(position) {
            data.frames[index] = frame.sanitized();
            let rt = data.frames[index].compile_rt();
            self.rt.store_frame(index, rt);
            return Some(index);
        }
        let index = data
            .positions
            .partition_point(|candidate| *candidate < position);
        data.frames.insert(index, frame.sanitized());
        data.positions.insert(index, position);
        let rt = data.compile_rt();
        self.rt.store(&rt);
        Some(index)
    }

    pub fn frame_position(&self, index: usize) -> Option<f32> {
        let data = self
            .data
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.is_positioned()
            .then(|| data.positions.get(index).copied())
            .flatten()
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
            let positioned = data.is_positioned();
            data.frames.remove(index);
            if positioned {
                data.positions.remove(index);
            }
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
    pub shape: f32,
    pub curve: WaveCurveRt,
    pub mix: f32,
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_VA_TABLE_FRAMES, VaTableData, VaTableState, nearest_frame_index, position_for_frame,
    };
    use crate::wave_curve::WaveCurveData;
    use truce::State;
    use truce_core::custom_state::State as StateTrait;

    #[test]
    fn table_positions_land_on_the_selected_frame() {
        assert_eq!(nearest_frame_index(0.0, 4), 0);
        assert_eq!(nearest_frame_index(0.25, 4), 0);
        assert_eq!(nearest_frame_index(0.5, 4), 1);
        assert_eq!(nearest_frame_index(1.0, 4), 3);
        assert!((position_for_frame(0, 4) - 0.25).abs() < f32::EPSILON);
        assert!((position_for_frame(3, 4) - 1.0).abs() < f32::EPSILON);
        assert_eq!(nearest_frame_index(position_for_frame(2, 8), 8), 2);
    }

    #[test]
    fn insert_frame_preserves_order_and_refuses_overflow() {
        let table = VaTableState::new();
        let first = WaveCurveData::default();
        let mut second = WaveCurveData::default();
        second.knots[0].value = 0.25;
        let generation = table.history_generation();
        assert_eq!(table.insert_frame(0, first.clone()), Some(0));
        assert_eq!(table.history_generation(), generation.wrapping_add(2));
        assert_eq!(table.insert_frame(0, second.clone()), Some(0));
        assert_eq!(table.snapshot().frames, vec![second, first]);
        while table.snapshot().frames.len() < MAX_VA_TABLE_FRAMES {
            assert!(
                table
                    .insert_frame(usize::MAX, WaveCurveData::default())
                    .is_some()
            );
        }
        let generation = table.history_generation();
        assert_eq!(table.insert_frame(0, WaveCurveData::default()), None);
        assert_eq!(table.history_generation(), generation);
        assert_eq!(table.snapshot().frames.len(), MAX_VA_TABLE_FRAMES);
    }

    fn distinct_curve(value: f32) -> WaveCurveData {
        let mut curve = WaveCurveData::default();
        curve.knots[0].value = value;
        curve
    }

    #[test]
    fn positioned_custom_key_is_local_to_its_canonical_neighbors() {
        let custom = distinct_curve(0.73);
        let rt = super::VaTableData {
            frames: vec![custom.clone()],
            positions: vec![0.5],
        }
        .compile_rt();

        let sine = rt.select(Default::default(), 0.9, 0.0);
        assert_eq!(sine.shape, 0.0);
        assert_eq!(sine.mix, 0.0);
        let triangle = rt.select(Default::default(), 0.9, 1.0 / 3.0);
        assert_eq!(triangle.shape, 1.0);
        assert_eq!(triangle.mix, 0.0);
        let before = rt.select(Default::default(), 0.9, 5.0 / 12.0);
        assert!((before.shape - 1.0).abs() < f32::EPSILON);
        assert!((before.mix - 0.5).abs() < 0.000_01);
        assert_eq!(before.curve, custom.compile_rt());
        let exact = rt.select(Default::default(), 0.9, 0.5);
        assert_eq!(exact.mix, 1.0);
        assert_eq!(exact.curve, custom.compile_rt());
        let after = rt.select(Default::default(), 0.9, 7.0 / 12.0);
        assert!((after.shape - 2.0).abs() < f32::EPSILON);
        assert!((after.mix - 0.5).abs() < 0.000_01);
        let saw = rt.select(Default::default(), 0.9, 2.0 / 3.0);
        assert_eq!(saw.shape, 2.0);
        assert_eq!(saw.mix, 0.0);
        let outside = rt.select(Default::default(), 0.9, 1.0 / 6.0);
        assert!((outside.shape - 0.5).abs() < f32::EPSILON);
        assert_eq!(outside.mix, 0.0);
    }

    #[test]
    fn positioned_insertion_preserves_positions_and_edits_only_the_target() {
        let table = VaTableState::new();
        let first = distinct_curve(0.2);
        let second = distinct_curve(0.8);
        assert_eq!(table.insert_positioned_frame(0.45, first.clone()), Some(0));
        assert_eq!(table.insert_positioned_frame(0.75, second.clone()), Some(1));
        let inserted = distinct_curve(-0.4);
        assert_eq!(
            table.insert_positioned_frame(0.55, inserted.clone()),
            Some(1)
        );
        let before_edit = table.snapshot();
        assert_eq!(before_edit.positions, vec![0.45, 0.55, 0.75]);
        assert!(table.replace_frame(1, distinct_curve(0.33)));
        let after_edit = table.snapshot();
        assert_eq!(after_edit.positions, before_edit.positions);
        assert_eq!(after_edit.frames[0], first);
        assert_eq!(after_edit.frames[2], second);
        assert_ne!(after_edit.frames[1], inserted);
    }

    #[test]
    fn positioned_insertion_rejects_factory_anchors_and_nonfinite_positions() {
        let table = VaTableState::new();
        for position in [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, f32::NAN] {
            assert_eq!(
                table.insert_positioned_frame(position, WaveCurveData::default()),
                None
            );
        }
        assert!(table.snapshot().frames.is_empty());
    }

    #[test]
    fn legacy_table_keeps_base_then_uniform_frame_mapping() {
        let custom = distinct_curve(0.61);
        let rt = super::VaTableData {
            frames: vec![custom.clone()],
            positions: Vec::new(),
        }
        .compile_rt();
        let half = rt.select(Default::default(), 0.5, 0.42);
        assert!((half.shape - 1.26).abs() < 0.000_01);
        assert!((half.mix - 0.5).abs() < f32::EPSILON);
        assert_eq!(half.curve, custom.compile_rt());
    }

    #[derive(Default, State)]
    struct LegacyVaTableData {
        frames: Vec<WaveCurveData>,
    }

    #[test]
    fn appended_positions_field_preserves_legacy_state_and_round_trips_positioned_state() {
        let legacy_curve = distinct_curve(0.19);
        let legacy_bytes = LegacyVaTableData {
            frames: vec![legacy_curve.clone()],
        }
        .serialize();
        let restored_legacy =
            VaTableData::deserialize(&legacy_bytes).expect("legacy keyed table decodes");
        assert_eq!(restored_legacy.frames, vec![legacy_curve]);
        assert!(restored_legacy.positions.is_empty());

        let positioned = VaTableData {
            frames: vec![distinct_curve(0.31), distinct_curve(0.72)],
            positions: vec![0.42, 0.78],
        };
        assert_eq!(
            VaTableData::deserialize(&positioned.serialize()),
            Some(positioned)
        );
    }

    #[test]
    fn positioned_custom_to_custom_interpolates_curves_without_procedural_mix() {
        let first = distinct_curve(0.18);
        let second = distinct_curve(0.82);
        let rt = VaTableData {
            frames: vec![first.clone(), second.clone()],
            positions: vec![0.42, 0.58],
        }
        .compile_rt();
        let selection = rt.select(Default::default(), 0.0, 0.5);
        assert_eq!(selection.mix, 1.0);
        assert_eq!(selection.shape, 1.5);
        assert_eq!(
            selection.curve,
            crate::wave_curve::WaveCurveRt::interpolate(
                first.compile_rt(),
                second.compile_rt(),
                (0.5 - 0.42) / (0.58 - 0.42),
            )
        );
    }

    #[test]
    fn positioned_duplicate_remove_and_initialize_do_not_redistribute_keys() {
        let table = VaTableState::new();
        let first = distinct_curve(0.2);
        let second = distinct_curve(0.8);
        assert_eq!(table.insert_positioned_frame(0.45, first.clone()), Some(0));
        assert_eq!(table.insert_positioned_frame(0.75, second.clone()), Some(1));
        let duplicate = table
            .duplicate_after(0, WaveCurveData::default())
            .expect("positioned duplicate");
        let duplicated = table.snapshot();
        assert_eq!(duplicated.positions[0], 0.45);
        assert_eq!(duplicated.positions[2], 0.75);
        assert!(duplicated.positions[duplicate] > 0.45);
        assert!(duplicated.positions[duplicate] < 2.0 / 3.0);
        assert_eq!(duplicated.frames[duplicate], first);

        assert!(table.remove_frame(duplicate));
        let removed = table.snapshot();
        assert_eq!(removed.positions, vec![0.45, 0.75]);
        assert_eq!(removed.frames, vec![first, second]);

        table.replace(VaTableData::default());
        assert_eq!(table.snapshot(), VaTableData::default());
    }
}
