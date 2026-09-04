use super::oscillator_bank::{
    ActiveOscillatorRenderSet, ActiveOscillatorSet, OscillatorDspConfig, ResynthPlaybackPlan,
    ResynthPlaybackPtr, StructuralOscillatorAbsoluteControl, StructuralOscillatorFrameControl,
};
use super::unison::{
    ALIGNMENT_EPSILON, AlignmentCandidate, EMPTY_ALIGNMENT_CANDIDATE, HARMONIC_CANDIDATE_CAP,
    UnisonAlignmentMode, UnisonLayout, UnisonSettings, build_harmonic_candidates,
    build_spatial_from_components, fill_unison_detune_positions, harmonic_candidate_upper,
    nearest_harmonic_candidate_lattice, stereo_square_weights,
};
use super::voice::{
    BLOCK_INTERNAL_SAMPLES, EnvelopeSettings, FACTOR3_BLOCK_INTERNAL_SAMPLES,
    LEGACY_OSCILLATOR_COUNT, MASTER_HEADROOM, POLYPHONY, POLYPHONY_U8, PitchModulationFrame,
    UnisonMotionFrame, VaVoice, VoiceSettings, midi_channel_matches, note_phase_seed,
    oscillator_stereo_seed, structural_ratio_bands, wrap_swarm_time,
};
use super::{MAX_UNISON, OscillatorMask, fast_exp2};
use crate::filters::{FilterCoefficients, FilterConfig};
use crate::generators::{
    AuxConfig, GeneratorRtGroup, GroupOutput, MAX_AUX_MODULES, MAX_FILTERS, MAX_OSCILLATORS,
    MAX_OUTPUT_PAIRS,
};
use crate::modulators::lfo::{LfoBank, VoiceLfoProgram, VoiceRouteFrame};
use crate::modulators::routing::{EXTRA_MODULATION_ROUTE_COUNT, MODULATION_ROUTE_COUNT};
use crate::{
    oscillators::{PhaseWarpMode, ProductionResynthArtifact},
    resynth_state::{ResynthRtPlanAck, ResynthRtUpdate},
};
use truce_simd::math::exp2_block;

#[derive(Clone, Copy, Default)]
struct HeldNote {
    note: u8,
    velocity: f32,
    channel: u8,
    voice_id: Option<i32>,
    per_note_bend: f32,
    per_note_timbre: Option<f32>,
}

#[derive(Clone, Copy)]
struct VoiceStructuralRoute {
    source: u8,
    factor: Option<u8>,
    amount: f32,
    target: crate::ResolvedModularTarget,
    generator_route: Option<u8>,
}

#[derive(Clone, Copy)]
pub(super) struct VoiceStructuralRouteFrame {
    entries: [Option<VoiceStructuralRoute>; MODULATION_ROUTE_COUNT],
    len: u8,
}

impl Default for VoiceStructuralRouteFrame {
    fn default() -> Self {
        Self {
            entries: [None; MODULATION_ROUTE_COUNT],
            len: 0,
        }
    }
}

impl VoiceStructuralRouteFrame {
    const fn route_count(&self) -> u8 {
        self.len
    }

    pub(super) fn single_filter_route(&self) -> Option<(u8, f32, u8, crate::FilterControl)> {
        let route = self.entries[0]?;
        let crate::ResolvedModularTarget::Filter { slot, control } = route.target else {
            return None;
        };
        (self.len == 1 && route.factor.is_none() && control != crate::FilterControl::Cutoff)
            .then_some((route.source, route.amount, slot, control))
    }

    fn filter_only(&self) -> bool {
        self.len != 0
            && self.entries[..usize::from(self.len)].iter().all(|route| {
                matches!(
                    route,
                    Some(VoiceStructuralRoute {
                        target: crate::ResolvedModularTarget::Filter { .. },
                        ..
                    })
                )
            })
    }

    fn oscillator_filter_only(&self) -> bool {
        self.len != 0
            && self.entries[..usize::from(self.len)].iter().all(|route| {
                matches!(
                    route,
                    Some(VoiceStructuralRoute {
                        target: crate::ResolvedModularTarget::Oscillator { .. }
                            | crate::ResolvedModularTarget::Filter { .. },
                        ..
                    })
                )
            })
    }

    pub(super) fn oscillator_gain_slot(&self) -> Option<usize> {
        let first = self.entries[0]?;
        let crate::ResolvedModularTarget::Oscillator {
            slot,
            control: crate::OscillatorControl::Level | crate::OscillatorControl::Pan,
        } = first.target
        else {
            return None;
        };
        self.entries[..usize::from(self.len)]
            .iter()
            .all(|route| {
                matches!(
                    route,
                    Some(VoiceStructuralRoute {
                        target: crate::ResolvedModularTarget::Oscillator {
                            slot: target,
                            control: crate::OscillatorControl::Level | crate::OscillatorControl::Pan,
                        },
                        ..
                    }) if *target == slot
                )
            })
            .then_some(usize::from(slot))
    }

    fn group_gain_pan_mask(&self) -> u8 {
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .fold(0, |mask, route| match route.target {
                crate::ResolvedModularTarget::Group {
                    index,
                    control: crate::GroupControl::Gain | crate::GroupControl::Pan,
                } => mask | (1 << index),
                _ => mask,
            })
    }

    fn group_envelope_mask(&self) -> u8 {
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .fold(0, |mask, route| match route.target {
                crate::ResolvedModularTarget::Group {
                    index,
                    control:
                        crate::GroupControl::AttackCurve
                        | crate::GroupControl::DecayCurve
                        | crate::GroupControl::ReleaseCurve,
                } => mask | (1 << index),
                _ => mask,
            })
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, source: u8, amount: f32, target: crate::ResolvedModularTarget) {
        let index = usize::from(self.len);
        if index == self.entries.len() {
            return;
        }
        self.entries[index] = Some(VoiceStructuralRoute {
            source,
            factor: None,
            amount,
            target,
            generator_route: None,
        });
        self.len += 1;
    }

    fn push_product(
        &mut self,
        source: u8,
        factor: u8,
        amount: f32,
        target: crate::ResolvedModularTarget,
    ) {
        let index = usize::from(self.len);
        if index == self.entries.len() {
            return;
        }
        self.entries[index] = Some(VoiceStructuralRoute {
            source,
            factor: Some(factor),
            amount,
            target,
            generator_route: None,
        });
        self.len += 1;
    }

    fn push_generator_depth(
        &mut self,
        source: u8,
        amount: f32,
        target_route: u8,
        target: crate::ResolvedModularTarget,
    ) {
        let index = usize::from(self.len);
        if index == self.entries.len() {
            return;
        }
        self.entries[index] = Some(VoiceStructuralRoute {
            source,
            factor: None,
            amount,
            target,
            generator_route: Some(target_route),
        });
        self.len += 1;
    }

    pub(super) fn generator_depth_target(&self) -> Option<u8> {
        let target = self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .find_map(|route| route.generator_route)?;
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .all(|route| route.generator_route.is_none_or(|route| route == target))
            .then_some(target)
    }

    pub(super) fn generator_depth_only(&self) -> bool {
        self.len != 0
            && self.entries[..usize::from(self.len)]
                .iter()
                .flatten()
                .all(|route| route.generator_route.is_some())
    }

    pub(super) fn has_regular_routes(&self) -> bool {
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .any(|route| route.generator_route.is_none())
    }

    pub(super) fn combined_generator_child(
        &self,
        target_route: u8,
    ) -> Option<(u8, f32, f32, crate::ResolvedModularTarget)> {
        let mut regular = None;
        let mut depth = None;
        for route in self.entries[..usize::from(self.len)].iter().flatten() {
            match route.generator_route {
                Some(target) if target == target_route && route.factor.is_none() => {
                    depth = Some((route.source, route.amount, route.target));
                }
                None if route.factor.is_none() => {
                    regular = Some((route.source, route.amount, route.target));
                }
                _ => return None,
            }
        }
        let (source, base, target) = regular?;
        let (depth_source, depth, _) = depth?;
        (source == depth_source).then_some((source, base, depth, target))
    }

    pub(super) fn generator_depth_amount(
        &self,
        values: &[f32; crate::modulators::lfo::LFO_COUNT],
        target: u8,
        base: f32,
    ) -> f32 {
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .filter(|route| route.generator_route == Some(target))
            .fold(base, |amount, route| {
                values[usize::from(route.source)].mul_add(route.amount, amount)
            })
            .clamp(-1.0, 1.0)
    }

    pub(super) fn evaluate(
        &self,
        values: &[f32; crate::modulators::lfo::LFO_COUNT],
        output: &mut crate::StructuralModulationFrame,
    ) {
        output.oscillator_mask = 0;
        output.group_mask = 0;
        output.group_envelope_mask = 0;
        output.filter_mask = 0;
        for index in 0..usize::from(self.len) {
            let Some(route) = self.entries[index] else {
                continue;
            };
            if route.generator_route.is_some() {
                continue;
            }
            match route.target {
                crate::ResolvedModularTarget::Oscillator { slot, .. } => {
                    if output.oscillator_mask & (1 << slot) == 0 {
                        output.oscillators[usize::from(slot)] =
                            crate::StructuralOscillatorDelta::default();
                    }
                    output.oscillator_mask |= 1 << slot;
                }
                crate::ResolvedModularTarget::Group { index, control } => {
                    if output.group_mask & (1 << index) == 0 {
                        output.groups[usize::from(index)] = crate::StructuralGroupDelta::default();
                    }
                    output.group_mask |= 1 << index;
                    output.group_envelope_mask |= u8::from(matches!(
                        control,
                        crate::GroupControl::AttackCurve
                            | crate::GroupControl::DecayCurve
                            | crate::GroupControl::ReleaseCurve
                    )) << index;
                }
                crate::ResolvedModularTarget::Filter { slot, .. } => {
                    if output.filter_mask & (1 << slot) == 0 {
                        output.filters[usize::from(slot)] = crate::StructuralFilterDelta::default();
                    }
                    output.filter_mask |= 1 << slot;
                }
                crate::ResolvedModularTarget::Aux { .. } => {}
            }
            let value = route
                .factor
                .map_or(values[usize::from(route.source)], |factor| {
                    values[usize::from(route.source)] * values[usize::from(factor)]
                });
            crate::runtime::render::accumulate_structural_modulation(
                output,
                route.target,
                value,
                route.amount,
            );
        }
    }

    pub(super) fn accumulate_phase_mod_depth<const SAMPLES: usize>(
        &self,
        values: &[f32; crate::modulators::lfo::LFO_COUNT],
        frame: usize,
        output: &mut [[f32; SAMPLES]; crate::generators::MAX_OSCILLATORS],
    ) {
        for route in self.entries[..usize::from(self.len)].iter().flatten() {
            if route.generator_route.is_some() {
                continue;
            }
            let crate::ResolvedModularTarget::Oscillator {
                slot,
                control: crate::OscillatorControl::PhaseModAmount,
            } = route.target
            else {
                continue;
            };
            let value = route
                .factor
                .map_or(values[usize::from(route.source)], |factor| {
                    values[usize::from(route.source)] * values[usize::from(factor)]
                });
            output[usize::from(slot)][frame] += value * route.amount.clamp(-1.0, 1.0);
        }
    }
}

#[derive(Clone, Copy)]
struct GeneratorStructuralRoute {
    route_index: u8,
    source: u8,
    target: u8,
    amount: f32,
    control: crate::OscillatorControl,
}

#[derive(Clone, Copy)]
struct GeneratorDepthRoute {
    source: u8,
    target_route: u8,
    amount: f32,
}

#[derive(Clone, Copy)]
struct GeneratorAuxRoute {
    route_index: u8,
    source: u8,
    amount: f32,
}

#[derive(Clone, Copy)]
struct GeneratorFilterRoute {
    route_index: u8,
    source: u8,
    slot: u8,
    amount: f32,
    control: crate::FilterControl,
}

#[derive(Clone, Copy)]
pub(super) struct GeneratorStructuralRouteFrame {
    entries: [Option<GeneratorStructuralRoute>; EXTRA_MODULATION_ROUTE_COUNT],
    next: [u8; EXTRA_MODULATION_ROUTE_COUNT],
    target_heads: [u8; MAX_OSCILLATORS],
    target_tails: [u8; MAX_OSCILLATORS],
    len: u8,
    target_mask: u32,
    source_mask: u32,
    filter_entries: [Option<GeneratorFilterRoute>; EXTRA_MODULATION_ROUTE_COUNT],
    filter_len: u8,
    aux_routes: [Option<GeneratorAuxRoute>; MAX_AUX_MODULES],
    aux_target_mask: u32,
    aux_source_mask: u32,
    aux_topology: [u16; MAX_AUX_MODULES],
    topology: [u16; EXTRA_MODULATION_ROUTE_COUNT],
    topology_len: u8,
    depth_topology: [u16; MODULATION_ROUTE_COUNT],
    depth_topology_len: u8,
    order: [u8; MAX_OSCILLATORS],
    feedback_routes: u64,
    depth_entries: [Option<GeneratorDepthRoute>; MODULATION_ROUTE_COUNT],
    depth_next: [u8; MODULATION_ROUTE_COUNT],
    depth_heads: [u8; MODULATION_ROUTE_COUNT],
    depth_tails: [u8; MODULATION_ROUTE_COUNT],
    depth_len: u8,
    depth_target_mask: u64,
    feedback_depth_routes: u64,
    feedback_source_mask: u32,
    fast_source_mask: u32,
    topology_revision: u32,
}

const NO_GENERATOR_ROUTE: u8 = u8::MAX;
const _: () = assert!(EXTRA_MODULATION_ROUTE_COUNT < NO_GENERATOR_ROUTE as usize);

impl Default for GeneratorStructuralRouteFrame {
    fn default() -> Self {
        Self {
            entries: [None; EXTRA_MODULATION_ROUTE_COUNT],
            next: [NO_GENERATOR_ROUTE; EXTRA_MODULATION_ROUTE_COUNT],
            target_heads: [NO_GENERATOR_ROUTE; MAX_OSCILLATORS],
            target_tails: [NO_GENERATOR_ROUTE; MAX_OSCILLATORS],
            len: 0,
            target_mask: 0,
            source_mask: 0,
            filter_entries: [None; EXTRA_MODULATION_ROUTE_COUNT],
            filter_len: 0,
            aux_routes: [None; MAX_AUX_MODULES],
            aux_target_mask: 0,
            aux_source_mask: 0,
            aux_topology: [0; MAX_AUX_MODULES],
            topology: [0; EXTRA_MODULATION_ROUTE_COUNT],
            topology_len: 0,
            depth_topology: [0; MODULATION_ROUTE_COUNT],
            depth_topology_len: 0,
            order: std::array::from_fn(|index| index as u8),
            feedback_routes: 0,
            depth_entries: [None; MODULATION_ROUTE_COUNT],
            depth_next: [NO_GENERATOR_ROUTE; MODULATION_ROUTE_COUNT],
            depth_heads: [NO_GENERATOR_ROUTE; MODULATION_ROUTE_COUNT],
            depth_tails: [NO_GENERATOR_ROUTE; MODULATION_ROUTE_COUNT],
            depth_len: 0,
            depth_target_mask: 0,
            feedback_depth_routes: 0,
            feedback_source_mask: 0,
            fast_source_mask: 0,
            topology_revision: 0,
        }
    }
}

impl GeneratorStructuralRouteFrame {
    fn clear(&mut self) {
        let mut targets = self.target_mask;
        while targets != 0 {
            let target = targets.trailing_zeros() as usize;
            targets &= targets - 1;
            self.target_heads[target] = NO_GENERATOR_ROUTE;
            self.target_tails[target] = NO_GENERATOR_ROUTE;
        }
        self.len = 0;
        self.target_mask = 0;
        self.source_mask = 0;
        self.filter_len = 0;
        let mut aux_targets = self.aux_target_mask;
        while aux_targets != 0 {
            let target = aux_targets.trailing_zeros() as usize;
            aux_targets &= aux_targets - 1;
            self.aux_routes[target] = None;
        }
        self.aux_target_mask = 0;
        self.aux_source_mask = 0;
        let mut depth_targets = self.depth_target_mask;
        while depth_targets != 0 {
            let route = depth_targets.trailing_zeros() as usize;
            depth_targets &= depth_targets - 1;
            self.depth_heads[route] = NO_GENERATOR_ROUTE;
            self.depth_tails[route] = NO_GENERATOR_ROUTE;
        }
        self.depth_len = 0;
        self.depth_target_mask = 0;
    }

    fn push(
        &mut self,
        source: u8,
        amount: f32,
        route_index: u8,
        target: crate::ResolvedModularTarget,
    ) {
        let (target, control) = match target {
            crate::ResolvedModularTarget::Oscillator { slot, control } => (slot, control),
            crate::ResolvedModularTarget::Filter { slot, control } => {
                let index = usize::from(self.filter_len);
                if index == self.filter_entries.len() {
                    return;
                }
                self.filter_entries[index] = Some(GeneratorFilterRoute {
                    route_index,
                    source,
                    slot,
                    amount,
                    control,
                });
                self.filter_len += 1;
                self.source_mask |= 1 << source;
                return;
            }
            crate::ResolvedModularTarget::Aux { slot } => {
                let slot = usize::from(slot);
                self.aux_routes[slot] = Some(GeneratorAuxRoute {
                    route_index,
                    source,
                    amount,
                });
                self.aux_target_mask |= 1 << slot;
                self.aux_source_mask |= 1 << source;
                self.source_mask |= 1 << source;
                return;
            }
            crate::ResolvedModularTarget::Group { .. } => return,
        };
        let index = usize::from(self.len);
        if index == self.entries.len() {
            return;
        }
        self.entries[index] = Some(GeneratorStructuralRoute {
            route_index,
            source,
            target,
            amount,
            control,
        });
        let target = usize::from(target);
        let tail = self.target_tails[target];
        if tail == NO_GENERATOR_ROUTE {
            self.target_heads[target] = self.len;
        } else {
            self.next[usize::from(tail)] = self.len;
        }
        self.next[index] = NO_GENERATOR_ROUTE;
        self.target_tails[target] = self.len;
        self.len += 1;
        self.target_mask |= 1 << target;
        self.source_mask |= 1 << source;
    }

    fn push_depth(&mut self, source: u8, amount: f32, target_route: u8) {
        let index = usize::from(self.depth_len);
        if index == self.depth_entries.len() {
            return;
        }
        self.depth_entries[index] = Some(GeneratorDepthRoute {
            source,
            target_route,
            amount,
        });
        let target = usize::from(target_route);
        let tail = self.depth_tails[target];
        if tail == NO_GENERATOR_ROUTE {
            self.depth_heads[target] = self.depth_len;
        } else {
            self.depth_next[usize::from(tail)] = self.depth_len;
        }
        self.depth_next[index] = NO_GENERATOR_ROUTE;
        self.depth_tails[target] = self.depth_len;
        self.depth_len += 1;
        self.depth_target_mask |= 1_u64 << target_route;
        self.source_mask |= 1 << source;
    }

    fn finish(&mut self) {
        let len = usize::from(self.len);
        let mut topology = [0_u16; EXTRA_MODULATION_ROUTE_COUNT];
        for (index, route) in self.entries[..len].iter().flatten().enumerate() {
            topology[index] = u16::from(route.source)
                | (u16::from(route.target) << 5)
                | (u16::from(route.route_index) << 10);
        }
        let depth_len = usize::from(self.depth_len);
        let mut depth_topology = [0_u16; MODULATION_ROUTE_COUNT];
        for (index, route) in self.depth_entries[..depth_len].iter().flatten().enumerate() {
            depth_topology[index] = u16::from(route.source) | (u16::from(route.target_route) << 5);
        }
        let mut aux_topology = [0_u16; MAX_AUX_MODULES];
        for (slot, route) in self.aux_routes.iter().copied().enumerate() {
            if let Some(route) = route {
                aux_topology[slot] =
                    u16::from(route.source) + 1 | ((u16::from(route.route_index) + 1) << 6);
            }
        }
        if self.topology_len == self.len
            && self.depth_topology_len == self.depth_len
            && self.topology[..len] == topology[..len]
            && self.depth_topology[..depth_len] == depth_topology[..depth_len]
            && self.aux_topology == aux_topology
        {
            return;
        }
        self.topology = topology;
        self.topology_len = self.len;
        self.depth_topology = depth_topology;
        self.depth_topology_len = self.depth_len;
        self.aux_topology = aux_topology;
        self.topology_revision = self.topology_revision.wrapping_add(1);

        let mut reach = [0_u32; MAX_OSCILLATORS];
        for route in self.entries[..len].iter().flatten() {
            reach[usize::from(route.source)] |= 1 << route.target;
        }
        let mut route_targets = [NO_GENERATOR_ROUTE; MODULATION_ROUTE_COUNT];
        for route in self.entries[..len].iter().flatten() {
            route_targets[usize::from(route.route_index)] = route.target;
        }
        for route in self.depth_entries[..depth_len].iter().flatten() {
            let target = route_targets[usize::from(route.target_route)];
            if target != NO_GENERATOR_ROUTE {
                reach[usize::from(route.source)] |= 1 << target;
            }
        }
        for intermediate in 0..MAX_OSCILLATORS {
            let intermediate_bit = 1 << intermediate;
            for source in 0..MAX_OSCILLATORS {
                if reach[source] & intermediate_bit != 0 {
                    reach[source] |= reach[intermediate];
                }
            }
        }

        self.feedback_routes = 0;
        self.feedback_depth_routes = 0;
        self.feedback_source_mask = 0;
        let mut dag = [0_u32; MAX_OSCILLATORS];
        for (index, route) in self.entries[..len].iter().flatten().enumerate() {
            let source = usize::from(route.source);
            let target = usize::from(route.target);
            let feedback = source == target
                || reach[source] & (1 << target) != 0 && reach[target] & (1 << source) != 0;
            if feedback {
                self.feedback_routes |= 1_u64 << index;
                self.feedback_source_mask |= 1 << source;
            } else {
                dag[source] |= 1 << target;
            }
        }
        for (index, route) in self.depth_entries[..depth_len].iter().flatten().enumerate() {
            let source = usize::from(route.source);
            let target = route_targets[usize::from(route.target_route)];
            if target == NO_GENERATOR_ROUTE {
                continue;
            }
            let target = usize::from(target);
            let feedback = source == target
                || reach[source] & (1 << target) != 0 && reach[target] & (1 << source) != 0;
            if feedback {
                self.feedback_depth_routes |= 1_u64 << index;
                self.feedback_source_mask |= 1 << source;
            } else {
                dag[source] |= 1 << target;
            }
        }

        let mut indegree = [0_u8; MAX_OSCILLATORS];
        for targets in dag {
            let mut targets = targets;
            while targets != 0 {
                let target = targets.trailing_zeros() as usize;
                targets &= targets - 1;
                indegree[target] += 1;
            }
        }
        let mut emitted = 0_u32;
        for output in &mut self.order {
            let source = (0..MAX_OSCILLATORS)
                .find(|&slot| emitted & (1 << slot) == 0 && indegree[slot] == 0)
                .expect("generator SCC condensation must be acyclic");
            *output = source as u8;
            emitted |= 1 << source;
            let mut targets = dag[source];
            while targets != 0 {
                let target = targets.trailing_zeros() as usize;
                targets &= targets - 1;
                indegree[target] -= 1;
            }
        }
    }

    pub(super) const fn order(&self) -> &[u8; MAX_OSCILLATORS] {
        &self.order
    }

    pub(super) const fn source_mask(&self) -> u32 {
        self.source_mask
    }

    pub(super) fn route_amount(&self, route_index: u8) -> Option<f32> {
        self.entries[..usize::from(self.len)]
            .iter()
            .flatten()
            .find(|route| route.route_index == route_index)
            .map(|route| route.amount)
            .or_else(|| {
                self.filter_entries
                    .iter()
                    .flatten()
                    .find(|route| route.route_index == route_index)
                    .map(|route| route.amount)
            })
            .or_else(|| {
                self.aux_routes
                    .iter()
                    .flatten()
                    .find(|route| route.route_index == route_index)
                    .map(|route| route.amount)
            })
    }

    pub(super) const fn aux_route(&self, target: usize) -> Option<(usize, f32)> {
        match self.aux_routes[target] {
            Some(route) => Some((route.source as usize, route.amount)),
            None => None,
        }
    }

    pub(super) const fn aux_source_mask(&self) -> u32 {
        self.aux_source_mask
    }

    pub(super) const fn aux_routes_active(&self) -> bool {
        self.aux_target_mask != 0
    }

    pub(super) const fn filter_routes_active(&self) -> bool {
        self.filter_len != 0
    }

    pub(super) fn ratio_filter_source_mask(
        &self,
        filters: &[FilterCoefficients; MAX_FILTERS],
    ) -> u32 {
        let mut mask = 0;
        for route in self.filter_entries[..usize::from(self.filter_len)]
            .iter()
            .flatten()
            .filter(|route| {
                filters[usize::from(route.slot)].is_ratio_brickwall()
                    && matches!(
                        route.control,
                        crate::FilterControl::Cutoff | crate::FilterControl::Shape
                    )
            })
        {
            mask |= 1 << route.source;
            let mut index = self.depth_heads[usize::from(route.route_index)];
            while index != NO_GENERATOR_ROUTE {
                let depth = self.depth_entries[usize::from(index)]
                    .expect("generator depth chain must reference a populated entry");
                mask |= 1 << depth.source;
                index = self.depth_next[usize::from(index)];
            }
        }
        mask
    }

    fn retain_supported_filter_routes(&mut self, filters: &[FilterCoefficients; MAX_FILTERS]) {
        let mut write = 0;
        for read in 0..usize::from(self.filter_len) {
            let route = self.filter_entries[read].expect("filter route prefix must be populated");
            if !filters[usize::from(route.slot)].is_ratio_brickwall()
                || matches!(
                    route.control,
                    crate::FilterControl::Cutoff | crate::FilterControl::Shape
                )
            {
                self.filter_entries[write] = Some(route);
                write += 1;
            }
        }
        if write == usize::from(self.filter_len) {
            return;
        }
        self.filter_entries[write..usize::from(self.filter_len)].fill(None);
        self.filter_len = write as u8;

        let mut route_mask = 0_u64;
        self.source_mask = 0;
        for route in self.entries[..usize::from(self.len)].iter().flatten() {
            route_mask |= 1_u64 << route.route_index;
            self.source_mask |= 1 << route.source;
        }
        for route in self.filter_entries[..write].iter().flatten() {
            route_mask |= 1_u64 << route.route_index;
            self.source_mask |= 1 << route.source;
        }
        for route in self.aux_routes.iter().flatten() {
            route_mask |= 1_u64 << route.route_index;
            self.source_mask |= 1 << route.source;
        }
        for route in self.depth_entries[..usize::from(self.depth_len)]
            .iter()
            .flatten()
        {
            if route_mask & (1_u64 << route.target_route) != 0 {
                self.source_mask |= 1 << route.source;
            }
        }
    }

    pub(super) fn single_filter_route(&self) -> Option<(u8, u8, u8, f32, crate::FilterControl)> {
        if self.filter_len != 1 {
            return None;
        }
        let route = self.filter_entries[0]?;
        (self.depth_heads[usize::from(route.route_index)] == NO_GENERATOR_ROUTE).then_some((
            route.route_index,
            route.source,
            route.slot,
            route.amount,
            route.control,
        ))
    }

    pub(super) const fn target_active(&self, target: usize) -> bool {
        self.target_mask & (1 << target) != 0
    }

    pub(super) const fn feedback_source_mask(&self) -> u32 {
        self.feedback_source_mask
    }

    fn enable_fast_muted_sources(&mut self, enabled: bool, audible_mask: u32) {
        self.fast_source_mask = if enabled {
            self.source_mask
                & !audible_mask
                & !self.target_mask
                & !self.feedback_source_mask
                & !self.aux_source_mask
        } else {
            0
        };
    }

    pub(super) const fn fast_source_mask(&self) -> u32 {
        self.fast_source_mask
    }

    pub(super) const fn topology_revision(&self) -> u32 {
        self.topology_revision
    }

    pub(super) fn gain_block_eligible(&self) -> bool {
        (self.len != 0 || self.filter_len != 0 || self.aux_target_mask != 0)
            && self.feedback_routes == 0
            && self.feedback_depth_routes == 0
            && self.entries[..usize::from(self.len)]
                .iter()
                .flatten()
                .all(|route| {
                    matches!(
                        route.control,
                        crate::OscillatorControl::Level
                            | crate::OscillatorControl::Pan
                            | crate::OscillatorControl::RingModAmount
                    )
                })
    }

    #[inline(always)]
    pub(super) fn single_gain_route(
        &self,
    ) -> Option<(u8, usize, usize, crate::OscillatorControl, f32)> {
        if self.len != 1 || self.depth_len != 0 || self.feedback_routes != 0 {
            return None;
        }
        let route = self.entries[0]?;
        matches!(
            route.control,
            crate::OscillatorControl::Level
                | crate::OscillatorControl::Pan
                | crate::OscillatorControl::RingModAmount
        )
        .then_some((
            route.route_index,
            usize::from(route.source),
            usize::from(route.target),
            route.control,
            route.amount.clamp(-1.0, 1.0),
        ))
    }

    pub(super) fn phase_block_eligible(&self) -> bool {
        self.filter_len == 0
            && self.len != 0
            && self.aux_target_mask == 0
            && self.feedback_routes == 0
            && self.feedback_depth_routes == 0
            && self.entries[..usize::from(self.len)]
                .iter()
                .flatten()
                .all(|route| route.control == crate::OscillatorControl::PhasePosition)
    }

    pub(super) fn mixed_phase_gain_routes(
        &self,
    ) -> Option<(usize, usize, f32, crate::OscillatorControl, f32)> {
        if self.filter_len != 0
            || self.len != 2
            || self.aux_target_mask != 0
            || self.depth_len != 0
            || self.feedback_routes != 0
            || self.feedback_depth_routes != 0
        {
            return None;
        }
        let first = self.entries[0]?;
        let second = self.entries[1]?;
        if first.source != second.source || first.target != second.target {
            return None;
        }
        let is_gain = |control| {
            matches!(
                control,
                crate::OscillatorControl::Level
                    | crate::OscillatorControl::Pan
                    | crate::OscillatorControl::RingModAmount
            )
        };
        let (phase, gain) = if first.control == crate::OscillatorControl::PhasePosition
            && is_gain(second.control)
        {
            (first, second)
        } else if second.control == crate::OscillatorControl::PhasePosition
            && is_gain(first.control)
        {
            (second, first)
        } else {
            return None;
        };
        let target_mask = 1 << phase.target;
        (target_mask & self.source_mask == 0).then_some((
            usize::from(phase.source),
            usize::from(phase.target),
            phase.amount.clamp(-1.0, 1.0),
            gain.control,
            gain.amount.clamp(-1.0, 1.0),
        ))
    }

    pub(super) fn pitch_block_eligible(&self) -> bool {
        self.filter_len == 0
            && self.len != 0
            && self.aux_target_mask == 0
            && self.feedback_routes == 0
            && self.feedback_depth_routes == 0
            && self.entries[..usize::from(self.len)]
                .iter()
                .flatten()
                .all(|route| {
                    matches!(
                        route.control,
                        crate::OscillatorControl::Transpose | crate::OscillatorControl::Cents
                    )
                })
    }

    pub(super) fn block_class(&self, fast_audio_rate_modulation: bool) -> u8 {
        if self.gain_block_eligible() {
            1
        } else if self.phase_block_eligible()
            || fast_audio_rate_modulation && self.mixed_phase_gain_routes().is_some()
        {
            2
        } else if self.pitch_block_eligible() {
            3
        } else {
            0
        }
    }

    #[inline(always)]
    pub(super) fn filter_delta(
        &self,
        slot: usize,
        source_values: &[f32; MAX_OSCILLATORS],
        route_amount: Option<(u8, f32)>,
    ) -> crate::StructuralFilterDelta {
        let mut delta = crate::StructuralFilterDelta::default();
        for route in self.filter_entries[..usize::from(self.filter_len)]
            .iter()
            .flatten()
            .filter(|route| usize::from(route.slot) == slot)
        {
            let mut amount = route_amount
                .filter(|(index, _)| *index == route.route_index)
                .map_or(route.amount, |(_, amount)| amount);
            let mut index = self.depth_heads[usize::from(route.route_index)];
            while index != NO_GENERATOR_ROUTE {
                let depth = self.depth_entries[usize::from(index)]
                    .expect("generator depth chain must reference a populated entry");
                amount += source_values[usize::from(depth.source)] * depth.amount;
                index = self.depth_next[usize::from(index)];
            }
            crate::runtime::render::accumulate_filter_modulation(
                &mut delta,
                route.control,
                source_values[usize::from(route.source)] * amount.clamp(-1.0, 1.0),
            );
        }
        delta
    }

    #[inline(always)]
    pub(super) fn accumulate_phase_block<const SAMPLES: usize>(
        &self,
        target: usize,
        source_values: &[[f32; SAMPLES]; MAX_OSCILLATORS],
        route_amounts: Option<(u8, &[f32])>,
        output: &mut [f32; SAMPLES],
    ) {
        debug_assert!(route_amounts.is_none_or(|(_, amounts)| amounts.len() == SAMPLES));
        let mut index = self.target_heads[target];
        while index != NO_GENERATOR_ROUTE {
            let route = self.entries[usize::from(index)]
                .expect("generator route chain must reference a populated entry");
            if route.control != crate::OscillatorControl::PhasePosition {
                index = self.next[usize::from(index)];
                continue;
            }
            if let Some((_, amounts)) =
                route_amounts.filter(|(target, _)| *target == route.route_index)
                && self.depth_heads[usize::from(route.route_index)] == NO_GENERATOR_ROUTE
            {
                for frame in 0..SAMPLES {
                    output[frame] +=
                        source_values[usize::from(route.source)][frame] * amounts[frame];
                }
            } else {
                for frame in 0..SAMPLES {
                    let amount = self.block_amount(
                        route,
                        source_values,
                        frame,
                        route_amounts.map(|(target, amounts)| (target, amounts[frame])),
                    );
                    output[frame] += source_values[usize::from(route.source)][frame] * amount;
                }
            }
            index = self.next[usize::from(index)];
        }
    }

    #[inline(always)]
    pub(super) fn accumulate_pitch_block<const SAMPLES: usize>(
        &self,
        target: usize,
        source_values: &[[f32; SAMPLES]; MAX_OSCILLATORS],
        route_amounts: Option<(u8, &[f32])>,
        output: &mut [f32; SAMPLES],
    ) {
        let mut index = self.target_heads[target];
        while index != NO_GENERATOR_ROUTE {
            let route = self.entries[usize::from(index)]
                .expect("generator route chain must reference a populated entry");
            for frame in 0..SAMPLES {
                let amount = self.block_amount(
                    route,
                    source_values,
                    frame,
                    route_amounts.map(|(target, amounts)| (target, amounts[frame])),
                ) * if route.control == crate::OscillatorControl::Transpose {
                    48.0
                } else {
                    1.0
                };
                output[frame] += source_values[usize::from(route.source)][frame] * amount;
            }
            index = self.next[usize::from(index)];
        }
    }

    #[inline(always)]
    pub(super) fn accumulate_block_frame<const SAMPLES: usize>(
        &self,
        target: usize,
        frame: usize,
        source_values: &[[f32; SAMPLES]; MAX_OSCILLATORS],
        route_amount: Option<(u8, f32)>,
        output: &mut crate::StructuralOscillatorDelta,
    ) -> (bool, f32) {
        if self.target_mask & (1 << target) == 0 {
            return (false, 1.0);
        }
        let mut active = false;
        let mut ring_gain = 1.0;
        let mut index = self.target_heads[target];
        while index != NO_GENERATOR_ROUTE {
            let route = self.entries[usize::from(index)]
                .expect("generator route chain must reference a populated entry");
            if !matches!(
                route.control,
                crate::OscillatorControl::Level
                    | crate::OscillatorControl::Pan
                    | crate::OscillatorControl::RingModAmount
            ) {
                index = self.next[usize::from(index)];
                continue;
            }
            active = true;
            let amount = self.block_amount(route, source_values, frame, route_amount);
            let source = source_values[usize::from(route.source)][frame];
            if route.control == crate::OscillatorControl::RingModAmount {
                let wet = amount.abs();
                ring_gain *= (1.0 - wet) + source * amount.signum() * wet;
            } else {
                crate::runtime::render::accumulate_oscillator_modulation(
                    output,
                    route.control,
                    source * amount,
                );
            }
            index = self.next[usize::from(index)];
        }
        (active, ring_gain)
    }

    #[inline(always)]
    pub(super) fn accumulate(
        &self,
        target: usize,
        source_values: &[f32; MAX_OSCILLATORS],
        feedback_values: &[f32; MAX_OSCILLATORS],
        feedback_valid: u32,
        rendered_mask: u32,
        route_amount: Option<(u8, f32)>,
        output: &mut crate::StructuralOscillatorDelta,
    ) -> (bool, f32) {
        if self.target_mask & (1 << target) == 0 {
            return (false, 1.0);
        }
        let mut active = false;
        let mut ring_gain = 1.0;
        let mut index = self.target_heads[target];
        while index != NO_GENERATOR_ROUTE {
            let route = self.entries[usize::from(index)]
                .expect("generator route chain must reference a populated entry");
            let route_index = usize::from(index);
            let feedback = self.feedback_routes & (1_u64 << route_index) != 0;
            if feedback || rendered_mask & (1 << route.source) != 0 {
                active = true;
                let amount = self.sample_amount(
                    route,
                    source_values,
                    feedback_values,
                    feedback_valid,
                    route_amount,
                );
                let source = if feedback {
                    if feedback_valid & (1 << route.source) != 0 {
                        feedback_values[usize::from(route.source)]
                    } else {
                        0.0
                    }
                } else {
                    source_values[usize::from(route.source)]
                };
                if route.control == crate::OscillatorControl::RingModAmount {
                    let wet = amount.abs();
                    ring_gain *= (1.0 - wet) + source * amount.signum() * wet;
                } else {
                    crate::runtime::render::accumulate_oscillator_modulation(
                        output,
                        route.control,
                        source * amount,
                    );
                }
            }
            index = self.next[usize::from(index)];
        }
        (active, ring_gain)
    }

    #[inline(always)]
    fn block_amount<const SAMPLES: usize>(
        &self,
        route: GeneratorStructuralRoute,
        source_values: &[[f32; SAMPLES]; MAX_OSCILLATORS],
        frame: usize,
        route_amount: Option<(u8, f32)>,
    ) -> f32 {
        let mut amount = route_amount
            .filter(|(target, _)| *target == route.route_index)
            .map_or(route.amount, |(_, amount)| amount);
        let mut index = self.depth_heads[usize::from(route.route_index)];
        while index != NO_GENERATOR_ROUTE {
            let depth = self.depth_entries[usize::from(index)]
                .expect("generator depth chain must reference a populated entry");
            amount += source_values[usize::from(depth.source)][frame] * depth.amount;
            index = self.depth_next[usize::from(index)];
        }
        amount.clamp(-1.0, 1.0)
    }

    #[inline(always)]
    fn sample_amount(
        &self,
        route: GeneratorStructuralRoute,
        source_values: &[f32; MAX_OSCILLATORS],
        feedback_values: &[f32; MAX_OSCILLATORS],
        feedback_valid: u32,
        route_amount: Option<(u8, f32)>,
    ) -> f32 {
        let mut amount = route_amount
            .filter(|(target, _)| *target == route.route_index)
            .map_or(route.amount, |(_, amount)| amount);
        let mut index = self.depth_heads[usize::from(route.route_index)];
        while index != NO_GENERATOR_ROUTE {
            let route_index = usize::from(index);
            let depth = self.depth_entries[route_index]
                .expect("generator depth chain must reference a populated entry");
            let source = if self.feedback_depth_routes & (1_u64 << route_index) != 0 {
                if feedback_valid & (1 << depth.source) != 0 {
                    feedback_values[usize::from(depth.source)]
                } else {
                    0.0
                }
            } else {
                source_values[usize::from(depth.source)]
            };
            amount += source * depth.amount;
            index = self.depth_next[route_index];
        }
        amount.clamp(-1.0, 1.0)
    }
}

fn apply_voice_settings(
    settings: &mut VoiceSettings,
    modulation: crate::modulators::lfo::ModulationFrame,
) {
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        let value = modulation.oscillator[oscillator];
        settings.modulate_oscillator(
            oscillator,
            value.pitch_semitones,
            value.shape,
            value.pulse_width,
            value.warp,
            value.custom_shape,
            value.level,
            value.pan,
        );
        settings
            .modulate_unison_detune_amount(oscillator, modulation.unison[oscillator].detune_amount);
    }
    settings.velocity_amount =
        (settings.velocity_amount + modulation.global.velocity).clamp(0.0, 1.0);
    settings.pressure_amount =
        (settings.pressure_amount + modulation.global.pressure).clamp(0.0, 1.0);
    settings.timbre_amount = (settings.timbre_amount + modulation.global.timbre).clamp(0.0, 1.0);
}

fn modulated_voice_envelope(
    base: EnvelopeSettings,
    modulation: crate::modulators::lfo::GlobalModulation,
) -> EnvelopeSettings {
    EnvelopeSettings {
        attack: (base.attack + modulation.attack).clamp(0.0, 8.0),
        decay: (base.decay + modulation.decay).clamp(0.0, 8.0),
        sustain: (base.sustain + modulation.sustain).clamp(0.0, 1.0),
        release: (base.release + modulation.release).clamp(0.0, 12.0),
        attack_curve: (base.attack_curve + modulation.attack_curve).clamp(-1.0, 1.0),
        decay_curve: (base.decay_curve + modulation.decay_curve).clamp(-1.0, 1.0),
        release_curve: (base.release_curve + modulation.release_curve).clamp(-1.0, 1.0),
        attack_curve_time: (base.attack_curve_time + modulation.attack_curve_time)
            .clamp(0.05, 0.95),
        decay_curve_time: (base.decay_curve_time + modulation.decay_curve_time).clamp(0.05, 0.95),
        release_curve_time: (base.release_curve_time + modulation.release_curve_time)
            .clamp(0.05, 0.95),
    }
}

fn add_unison_modulation(
    left: crate::modulators::lfo::UnisonModulation,
    right: crate::modulators::lfo::UnisonModulation,
) -> crate::modulators::lfo::UnisonModulation {
    crate::modulators::lfo::UnisonModulation {
        detune_amount: left.detune_amount + right.detune_amount,
        detune_cents: left.detune_cents + right.detune_cents,
        harmonic_align: left.harmonic_align + right.harmonic_align,
        stereo: left.stereo + right.stereo,
        phase_random: left.phase_random + right.phase_random,
        curve: left.curve + right.curve,
        jitter_amount: left.jitter_amount + right.jitter_amount,
        jitter_rate_normalized: left.jitter_rate_normalized + right.jitter_rate_normalized,
        stereo_x: left.stereo_x + right.stereo_x,
        stereo_y: left.stereo_y + right.stereo_y,
        weight: left.weight + right.weight,
        pan_center: left.pan_center + right.pan_center,
        pan_left: left.pan_left + right.pan_left,
        pan_right: left.pan_right + right.pan_right,
        pan_center_x: left.pan_center_x + right.pan_center_x,
    }
}

#[inline(always)]
fn voice_output_gain(db: f32) -> f32 {
    2.0_f32.powf(db.clamp(-96.0, 24.0) / 6.020_6)
}

fn apply_voice_unison_motion(
    voice: &mut VaVoice,
    base: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    modulation: &[crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
) {
    for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
        let delta = modulation[oscillator];
        let rate_scale = if delta.jitter_rate_normalized == 0.0 {
            1.0
        } else {
            5_000.0_f32.powf(delta.jitter_rate_normalized.clamp(-1.0, 1.0))
        };
        let mut settings = base[oscillator].modulated(delta);
        settings = settings.with_motion(
            settings.phase_random(),
            settings.swarm_amount(),
            (base[oscillator].swarm_rate() * rate_scale).clamp(0.02, 100.0),
        );
        if oscillator == 0 {
            voice.configure_unison_motion(settings);
        } else {
            voice.configure_secondary_unison_motion(oscillator, settings);
        }
    }
}

fn merge_voice_structural_control(
    base: &StructuralOscillatorFrameControl,
    modulation: &crate::StructuralModulationFrame,
    bank: &ActiveOscillatorSet,
) -> StructuralOscillatorFrameControl {
    let mut output = *base;
    let mut mask = modulation.oscillator_mask;
    while mask != 0 {
        let slot = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        let Some(config) = bank.configured[slot] else {
            continue;
        };
        let delta = modulation.oscillators[slot];
        let target = if output.mask & (1 << slot) != 0 {
            output.slots[slot]
        } else {
            let level = config.level.clamp(0.0, 1.0);
            let pan = config.pan.clamp(-1.0, 1.0);
            StructuralOscillatorAbsoluteControl {
                shape: config.shape,
                pulse_width: config.pulse_width,
                pitch_ratio: fast_exp2(
                    (config.transpose.clamp(-48.0, 48.0)
                        + config.cents.clamp(-100.0, 100.0) * 0.01)
                        / 12.0,
                ),
                phase_position: config.phase_position,
                phase_warp_amount: config.phase_warp_amount,
                phase_mod_amount: config.phase_mod_amount,
                left_gain: level * (1.0 - pan).sqrt(),
                right_gain: level * (1.0 + pan).sqrt(),
                unison_jitter: config.unison_jitter,
                unison_rate: config.unison_rate,
                stereo_x: 0.0,
                stereo_y: 0.0,
                grain_tune: 0.0,
                grain_stereo: 0.0,
                rich_balance: 0.0,
                rich_formant: 0.0,
                rich_air: 0.0,
                rich_diffuse: 0.0,
                rich_dynamic: 0.0,
            }
        };
        let target = apply_voice_structural_delta(target, delta, !config.positioned_wave);
        output.slots[slot] = target;
        output.mask |= 1 << slot;
    }
    output
}

pub(super) fn merge_voice_structural_block_control(
    base: &StructuralOscillatorFrameControl,
    modulation: &crate::StructuralModulationFrame,
    bank: &ActiveOscillatorRenderSet,
) -> StructuralOscillatorFrameControl {
    let mut output = *base;
    let mut mask = modulation.oscillator_mask;
    while mask != 0 {
        let slot = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        if output.mask & (1 << slot) == 0 {
            let oscillator = &bank.entry(slot).current;
            output.slots[slot] = StructuralOscillatorAbsoluteControl {
                shape: oscillator.shape,
                pulse_width: oscillator.pulse_width,
                pitch_ratio: oscillator.pitch_ratio,
                phase_position: oscillator.phase_position,
                phase_warp_amount: oscillator.phase_warp.amount,
                phase_mod_amount: oscillator.phase_mod_amount,
                left_gain: oscillator.left_gain,
                right_gain: oscillator.right_gain,
                unison_jitter: oscillator.unison_jitter,
                unison_rate: oscillator.jitter_rate_hz,
                stereo_x: 0.0,
                stereo_y: 0.0,
                grain_tune: 0.0,
                grain_stereo: 0.0,
                rich_balance: 0.0,
                rich_formant: 0.0,
                rich_air: 0.0,
                rich_diffuse: 0.0,
                rich_dynamic: 0.0,
            };
            output.mask |= 1 << slot;
            output.gain_only_mask |= 1 << slot;
        }
        let mut non_gain_delta = modulation.oscillators[slot];
        non_gain_delta.level = 0.0;
        non_gain_delta.pan = 0.0;
        if non_gain_delta != crate::StructuralOscillatorDelta::default() {
            output.gain_only_mask &= !(1 << slot);
        }
        output.slots[slot] = apply_voice_structural_delta(
            output.slots[slot],
            modulation.oscillators[slot],
            !bank.entry(slot).current.positioned_wave,
        );
    }
    output
}

pub(super) fn apply_voice_structural_delta(
    mut target: StructuralOscillatorAbsoluteControl,
    delta: crate::StructuralOscillatorDelta,
    apply_shape: bool,
) -> StructuralOscillatorAbsoluteControl {
    if apply_shape {
        target.shape = (target.shape + delta.shape).clamp(0.0, 3.0);
    }
    target.pulse_width = (target.pulse_width + delta.pulse_width).clamp(0.03, 0.97);
    if delta.pitch_semitones != 0.0 {
        target.pitch_ratio = (target.pitch_ratio
            * fast_exp2(delta.pitch_semitones.clamp(-48.0, 48.0) / 12.0))
        .clamp(1.0 / 256.0, 256.0);
    }
    if delta.phase_position != 0.0 {
        target.phase_position = (target.phase_position + delta.phase_position).rem_euclid(1.0);
    }
    target.phase_warp_amount = (target.phase_warp_amount + delta.warp).clamp(0.0, 1.0);
    target.phase_mod_amount = (target.phase_mod_amount + delta.phase_mod_amount).clamp(-1.0, 1.0);
    if delta.level != 0.0 || delta.pan != 0.0 {
        let left_power = target.left_gain * target.left_gain;
        let right_power = target.right_gain * target.right_gain;
        let current_level = (left_power + right_power).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
        let current_pan = (right_power - left_power) / (right_power + left_power).max(f32::EPSILON);
        let level = (current_level + delta.level).clamp(0.0, 1.0);
        let pan = (current_pan + delta.pan).clamp(-1.0, 1.0);
        target.left_gain = level * (1.0 - pan).sqrt();
        target.right_gain = level * (1.0 + pan).sqrt();
    }
    target.unison_jitter = (target.unison_jitter + delta.unison_jitter).clamp(0.0, 1.0);
    target.unison_rate = (target.unison_rate + delta.unison_rate).clamp(0.0, 1.0);
    target.stereo_x += delta.stereo_x;
    target.stereo_y += delta.stereo_y;
    target.grain_tune += delta.grain_tune;
    target.grain_stereo += delta.grain_stereo;
    target.rich_balance += delta.rich_balance;
    target.rich_formant += delta.rich_formant;
    target.rich_air += delta.rich_air;
    target.rich_diffuse += delta.rich_diffuse;
    target.rich_dynamic += delta.rich_dynamic;
    target
}

fn merge_voice_filter_coefficients(
    base: &[FilterConfig; MAX_FILTERS],
    shared: &[FilterCoefficients; MAX_FILTERS],
    modulation: &crate::StructuralModulationFrame,
    sample_rate: f32,
) -> [FilterCoefficients; MAX_FILTERS] {
    let mut output = *shared;
    let mut mask = modulation.filter_mask;
    while mask != 0 {
        let slot = mask.trailing_zeros() as usize;
        mask &= mask - 1;
        let delta = modulation.filters[slot];
        output[slot] = voice_filter_coefficient(base[slot], shared[slot], delta, sample_rate);
    }
    output
}

pub(super) fn voice_filter_coefficient(
    base: FilterConfig,
    shared: FilterCoefficients,
    delta: crate::StructuralFilterDelta,
    sample_rate: f32,
) -> FilterCoefficients {
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared;
    }
    if delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_cutoff(delta.cutoff_octaves);
    }
    if delta.cutoff_octaves == 0.0 && delta.slope == 0.0 && delta.morph == 0.0 && delta.shape == 0.0
    {
        return shared.modulated_resonance(delta.resonance_octaves);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_slope(delta.slope);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_morph(delta.morph);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
    {
        return shared.modulated_shape(delta.shape);
    }
    base.modulated(
        delta.cutoff_octaves,
        delta.resonance_octaves,
        delta.slope,
        delta.morph,
        delta.shape,
    )
    .coefficients(sample_rate)
}

#[inline(always)]
pub(super) fn generator_filter_coefficient(
    shared: FilterCoefficients,
    delta: crate::StructuralFilterDelta,
) -> FilterCoefficients {
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared;
    }
    if delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_cutoff(delta.cutoff_octaves);
    }
    if delta.cutoff_octaves == 0.0 && delta.slope == 0.0 && delta.morph == 0.0 && delta.shape == 0.0
    {
        return shared.modulated_resonance(delta.resonance_octaves);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.morph == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_slope(delta.slope);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.shape == 0.0
    {
        return shared.modulated_morph(delta.morph);
    }
    if delta.cutoff_octaves == 0.0
        && delta.resonance_octaves == 0.0
        && delta.slope == 0.0
        && delta.morph == 0.0
    {
        return shared.modulated_shape(delta.shape);
    }
    shared
        .modulated_cutoff(delta.cutoff_octaves)
        .modulated_resonance(delta.resonance_octaves)
        .modulated_slope(delta.slope)
        .modulated_morph(delta.morph)
        .modulated_shape(delta.shape)
}

#[inline(always)]
fn modulated_group_envelope(
    mut envelope: EnvelopeSettings,
    voice: crate::StructuralGroupDelta,
    shared: crate::StructuralGroupDelta,
) -> EnvelopeSettings {
    envelope.attack_curve =
        (envelope.attack_curve + voice.attack_curve + shared.attack_curve).clamp(-1.0, 1.0);
    envelope.decay_curve =
        (envelope.decay_curve + voice.decay_curve + shared.decay_curve).clamp(-1.0, 1.0);
    envelope.release_curve =
        (envelope.release_curve + voice.release_curve + shared.release_curve).clamp(-1.0, 1.0);
    envelope
}

#[inline(always)]
fn apply_group_gain_pan(
    stem: &mut (f32, f32),
    output: GroupOutput,
    voice: crate::StructuralGroupDelta,
    shared: crate::StructuralGroupDelta,
) {
    let gain = (output.gain + voice.gain + shared.gain).clamp(0.0, 2.0);
    let pan = (output.pan + voice.pan + shared.pan).clamp(-1.0, 1.0);
    stem.0 *= gain * (1.0 - pan).sqrt();
    stem.1 *= gain * (1.0 + pan).sqrt();
}

pub(super) struct UnisonFrameControl {
    pub(super) pitch_correction: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub(super) dynamic_detune_positions: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub(super) dynamic_position_mask: OscillatorMask,
    pub(super) active_mask: OscillatorMask,
    pub(super) spatial: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
    pub(super) spatial_mask: OscillatorMask,
    pub(super) spatial_left: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub(super) spatial_right: [[f32; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
    pub(super) spatial_gain: [f32; LEGACY_OSCILLATOR_COUNT],
    pub(super) spatial_shared_mask: OscillatorMask,
    pub(super) exponents: [f32; MAX_UNISON],
}

impl UnisonFrameControl {
    pub(super) const NEUTRAL: Self = Self {
        pitch_correction: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
        dynamic_detune_positions: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
        dynamic_position_mask: 0,
        active_mask: 0,
        spatial: [crate::modulators::lfo::UnisonModulation {
            detune_amount: 0.0,
            detune_cents: 0.0,
            harmonic_align: 0.0,
            stereo: 0.0,
            phase_random: 0.0,
            curve: 0.0,
            jitter_amount: 0.0,
            jitter_rate_normalized: 0.0,
            stereo_x: 0.0,
            stereo_y: 0.0,
            weight: 0.0,
            pan_center: 0.0,
            pan_left: 0.0,
            pan_right: 0.0,
            pan_center_x: 0.0,
        }; LEGACY_OSCILLATOR_COUNT],
        spatial_mask: 0,
        spatial_left: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
        spatial_right: [[0.0; MAX_UNISON]; LEGACY_OSCILLATOR_COUNT],
        spatial_gain: [0.0; LEGACY_OSCILLATOR_COUNT],
        spatial_shared_mask: 0,
        exponents: [0.0; MAX_UNISON],
    };
}

pub struct PolySynth {
    pub(super) voices: Box<[VaVoice]>,
    pub(super) envelope: EnvelopeSettings,
    output_group_envelopes: [EnvelopeSettings; MAX_OUTPUT_PAIRS],
    output_group_midi_channels: [u8; MAX_OUTPUT_PAIRS],
    output_group_count: u8,
    output_group_active_mask: u8,
    output_group_envelopes_enabled: bool,
    output_group_envelope_modulation_mask: u8,
    audible_oscillator_mask: u32,
    pub(super) sample_rate: f32,
    age: u64,
    pub(super) active_count: u8,
    sustain: [bool; 16],
    parameter_bend: f32,
    pitch_bend: [f32; 16],
    per_note_bend: [f32; POLYPHONY],
    per_note_timbre: [Option<f32>; POLYPHONY],
    timbre: [f32; 16],
    latest_stereo_seed: [f32; LEGACY_OSCILLATOR_COUNT],
    pub(super) swarm_time: f64,
    pub(super) swarm_step: f64,
    pub(super) secondary_swarm_time: [f64; LEGACY_OSCILLATOR_COUNT - 1],
    pub(super) secondary_swarm_step: [f64; LEGACY_OSCILLATOR_COUNT - 1],
    enabled_oscillator_mask: OscillatorMask,
    pub(super) oscillator_bank: Box<ActiveOscillatorSet>,
    // Boxed once so every render setting can point at a stable two-layer plan.
    resynth_playback: Box<[ResynthPlaybackPlan]>,
    // Monitor identities advance only at callback/block publication boundaries;
    // they never participate in the audio sample hot path.
    resynth_publish_frame: u64,
    resynth_audio_frame: u64,
    unison_settings: [UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    unison_templates: [UnisonLayout; LEGACY_OSCILLATOR_COUNT],
    harmonic_candidates: [[AlignmentCandidate; HARMONIC_CANDIDATE_CAP]; 4],
    harmonic_candidate_counts: [u8; 4],
    phase_warp_mode: [PhaseWarpMode; LEGACY_OSCILLATOR_COUNT],
    voice_mode: u8,
    reference_tuning_hz: f32,
    transpose_semitones: f32,
    glide_time: f32,
    mono_stack: [HeldNote; POLYPHONY],
    mono_stack_len: u8,
    frame_control_cache: Option<Box<UnisonFrameControl>>,
    frame_control_modulation: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
    frame_control_valid: bool,
    pitch_block_controls: Box<[PitchModulationFrame]>,
    voice_lfo_program: Box<VoiceLfoProgram>,
    voice_route_frame: VoiceRouteFrame,
    voice_structural_route_frame: VoiceStructuralRouteFrame,
    generator_structural_route_frame: GeneratorStructuralRouteFrame,
    voice_filter_configs: [FilterConfig; MAX_FILTERS],
    aux_configs: [AuxConfig; MAX_AUX_MODULES],
}

fn settle_resynth_plans(plans: &mut [ResynthPlaybackPlan]) {
    for plan in plans {
        plan.settle_artifact_when_idle();
    }
}

fn retire_finished_voice(
    voice: &VaVoice,
    active_count: &mut u8,
    plans: &mut [ResynthPlaybackPlan],
) {
    if !voice.active() {
        *active_count -= 1;
        if *active_count == 0 {
            settle_resynth_plans(plans);
        }
    }
}

fn set_voice_swarm_clocks(
    voice: &mut VaVoice,
    settings: VoiceSettings,
    swarm_time: f64,
    secondary_swarm_time: [f64; LEGACY_OSCILLATOR_COUNT - 1],
) {
    voice.set_swarm_clock(swarm_time as f32);
    for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
        if settings.oscillator(secondary + 1).enabled {
            voice.set_secondary_swarm_clock(secondary + 1, secondary_swarm_time[secondary] as f32);
        }
    }
}

fn active_voices_mut(
    voices: &mut [VaVoice],
    active_count: u8,
) -> impl Iterator<Item = &mut VaVoice> {
    voices
        .iter_mut()
        .filter(|voice| voice.active())
        .take(usize::from(active_count))
}

impl Default for PolySynth {
    fn default() -> Self {
        let mut harmonic_candidates = [[EMPTY_ALIGNMENT_CANDIDATE; HARMONIC_CANDIDATE_CAP]; 4];
        let mut harmonic_candidate_counts = [0; 4];
        for index in 0..4 {
            let (candidates, count) =
                build_harmonic_candidates(UnisonAlignmentMode::from_index(index as u8));
            harmonic_candidates[index] = candidates;
            harmonic_candidate_counts[index] = count as u8;
        }
        Self {
            voices: (0..POLYPHONY)
                .map(|_| VaVoice::default())
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            envelope: EnvelopeSettings::default(),
            output_group_envelopes: [EnvelopeSettings::default(); MAX_OUTPUT_PAIRS],
            output_group_midi_channels: [0; MAX_OUTPUT_PAIRS],
            output_group_count: 1,
            output_group_active_mask: 1,
            output_group_envelopes_enabled: false,
            output_group_envelope_modulation_mask: 0,
            audible_oscillator_mask: u32::MAX,
            sample_rate: 44_100.0,
            age: 0,
            active_count: 0,
            sustain: [false; 16],
            parameter_bend: 0.0,
            pitch_bend: [0.0; 16],
            per_note_bend: [0.0; POLYPHONY],
            per_note_timbre: [None; POLYPHONY],
            timbre: [0.5; 16],
            latest_stereo_seed: [0.5; LEGACY_OSCILLATOR_COUNT],
            swarm_time: 0.0,
            swarm_step: 0.7 / 44_100.0,
            secondary_swarm_time: [0.0; LEGACY_OSCILLATOR_COUNT - 1],
            secondary_swarm_step: [0.7 / 44_100.0; LEGACY_OSCILLATOR_COUNT - 1],
            enabled_oscillator_mask: 1,
            oscillator_bank: Box::new(ActiveOscillatorSet::default()),
            resynth_playback: (0..MAX_OSCILLATORS)
                .map(|_| ResynthPlaybackPlan::default())
                .collect(),
            resynth_publish_frame: 0,
            resynth_audio_frame: 0,
            unison_settings: std::array::from_fn(|_| UnisonSettings::new(1, 0.0, 0.0, 1.0, 0.0)),
            unison_templates: std::array::from_fn(|_| UnisonLayout::default()),
            harmonic_candidates,
            harmonic_candidate_counts,
            phase_warp_mode: [PhaseWarpMode::None; LEGACY_OSCILLATOR_COUNT],
            voice_mode: POLYPHONY_U8,
            reference_tuning_hz: 440.0,
            transpose_semitones: 0.0,
            glide_time: 0.0,
            mono_stack: [HeldNote::default(); POLYPHONY],
            mono_stack_len: 0,
            frame_control_cache: Some(Box::new(UnisonFrameControl::NEUTRAL)),
            frame_control_modulation: [crate::modulators::lfo::UnisonModulation::default();
                LEGACY_OSCILLATOR_COUNT],
            frame_control_valid: false,
            pitch_block_controls: vec![PitchModulationFrame::default(); BLOCK_INTERNAL_SAMPLES]
                .into_boxed_slice(),
            voice_lfo_program: Box::new(VoiceLfoProgram::default()),
            voice_route_frame: VoiceRouteFrame::default(),
            voice_structural_route_frame: VoiceStructuralRouteFrame::default(),
            generator_structural_route_frame: GeneratorStructuralRouteFrame::default(),
            voice_filter_configs: [FilterConfig::default(); MAX_FILTERS],
            aux_configs: [AuxConfig::default(); MAX_AUX_MODULES],
        }
    }
}

impl PolySynth {
    pub(crate) fn sync_voice_lfos(&mut self, lfos: &LfoBank, source_mask: u64) {
        let previous_mask = self.voice_lfo_program.polyphonic_mask();
        lfos.sync_voice_program(&mut self.voice_lfo_program, source_mask);
        let added = self.voice_lfo_program.polyphonic_mask() & !previous_mask;
        if added != 0 {
            for voice in &mut self.voices {
                if voice.active() {
                    voice.activate_modulation_sources(&self.voice_lfo_program, added);
                }
            }
        }
    }

    pub(crate) const fn voice_modulation_active(&self) -> bool {
        self.voice_route_frame.active() || self.voice_structural_route_frame.route_count() != 0
    }

    pub(crate) fn voice_group_modulation_mask(&self) -> u8 {
        self.voice_structural_route_frame.group_gain_pan_mask()
    }

    fn retain_output_group_envelope_modulation(&mut self, mask: u8) {
        let stale = self.output_group_envelope_modulation_mask & !mask;
        if stale == 0 {
            return;
        }
        let count = usize::from(self.output_group_count);
        for voice in &mut self.voices {
            let mut groups = stale;
            while groups != 0 {
                let group = groups.trailing_zeros() as usize;
                groups &= groups - 1;
                if group < count {
                    voice
                        .configure_output_group_envelope(group, self.output_group_envelopes[group]);
                }
            }
        }
        self.output_group_envelope_modulation_mask &= mask;
    }

    fn restore_output_group_envelopes(&mut self) {
        self.retain_output_group_envelope_modulation(0);
    }

    fn configure_shared_output_group_envelopes(
        &mut self,
        structural: &crate::StructuralModulationFrame,
    ) {
        if structural.group_envelope_mask == 0 {
            self.restore_output_group_envelopes();
            return;
        }
        self.retain_output_group_envelope_modulation(structural.group_envelope_mask);
        let count = usize::from(self.output_group_count);
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let mut mask = structural.group_envelope_mask;
            while mask != 0 {
                let group = mask.trailing_zeros() as usize;
                mask &= mask - 1;
                if group < count {
                    voice.configure_output_group_envelope(
                        group,
                        modulated_group_envelope(
                            self.output_group_envelopes[group],
                            crate::StructuralGroupDelta::default(),
                            structural.groups[group],
                        ),
                    );
                }
            }
        }
        self.output_group_envelope_modulation_mask = structural.group_envelope_mask;
    }

    pub(crate) fn voice_structural_modulation_block_eligible(
        &self,
        settings: VoiceSettings,
    ) -> bool {
        self.structural_modulation_block_eligible(settings)
            && self.voice_lfo_program.active()
            && !self.voice_route_frame.active()
            && self.voice_structural_route_frame.oscillator_filter_only()
    }

    pub(super) fn voice_structural_job_context(
        &self,
    ) -> (&VoiceLfoProgram, VoiceStructuralRouteFrame) {
        (&self.voice_lfo_program, self.voice_structural_route_frame)
    }

    pub(super) fn voice_structural_workload(&self) -> (u32, u8) {
        (
            self.voice_lfo_program.active_source_count(),
            self.voice_structural_route_frame.route_count(),
        )
    }

    pub(crate) fn voice_filter_modulation_only(&self) -> bool {
        self.voice_lfo_program.active()
            && !self.voice_route_frame.active()
            && self.voice_structural_route_frame.filter_only()
    }

    pub(crate) const fn voice_polyphonic_mask(&self) -> u64 {
        self.voice_lfo_program.polyphonic_mask()
    }

    pub(crate) fn set_voice_lfo_control(&mut self, index: usize, rate: f32, phase: f32) {
        self.voice_lfo_program
            .set_dynamic_control(index, rate, phase);
    }

    pub(crate) fn voice_lfo_snapshot(
        &self,
    ) -> Option<(
        [f32; crate::modulators::lfo::LFO_COUNT],
        [f32; crate::modulators::lfo::LFO_COUNT],
        u64,
    )> {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .max_by_key(|voice| voice.age)
            .map(|voice| voice.modulation.snapshot(&self.voice_lfo_program))
    }

    #[cfg(test)]
    pub(crate) fn evaluate_voice_structural_routes_for_test(
        &self,
        values: &[f32; crate::modulators::lfo::LFO_COUNT],
    ) -> crate::StructuralModulationFrame {
        let mut output = crate::StructuralModulationFrame::default();
        self.voice_structural_route_frame
            .evaluate(values, &mut output);
        output
    }

    pub(crate) fn begin_voice_modulation_frame(&mut self) {
        self.voice_route_frame.clear();
        self.voice_structural_route_frame.clear();
        self.generator_structural_route_frame.clear();
    }

    pub(crate) fn push_voice_structural_route(
        &mut self,
        source: u8,
        amount: f32,
        target: crate::ResolvedModularTarget,
    ) {
        self.voice_structural_route_frame
            .push(source, amount, target);
    }

    pub(crate) fn push_voice_structural_product_route(
        &mut self,
        source: u8,
        factor: u8,
        amount: f32,
        target: crate::ResolvedModularTarget,
    ) {
        self.voice_structural_route_frame
            .push_product(source, factor, amount, target);
    }

    pub(crate) fn push_voice_generator_depth_route(
        &mut self,
        source: u8,
        amount: f32,
        target_route: u8,
        target: crate::ResolvedModularTarget,
    ) {
        self.voice_structural_route_frame.push_generator_depth(
            source,
            amount,
            target_route,
            target,
        );
    }

    pub(crate) fn push_generator_structural_route(
        &mut self,
        source: u8,
        amount: f32,
        route_index: u8,
        target: crate::ResolvedModularTarget,
    ) {
        self.generator_structural_route_frame
            .push(source, amount, route_index, target);
    }

    pub(crate) fn push_generator_depth_route(&mut self, source: u8, amount: f32, target_route: u8) {
        self.generator_structural_route_frame
            .push_depth(source, amount, target_route);
    }

    pub(crate) fn finish_voice_modulation_frame(&mut self) {
        self.generator_structural_route_frame.finish();
    }

    pub(crate) fn configure_voice_filters(
        &mut self,
        configs: &[FilterConfig; MAX_FILTERS],
        mask: u32,
    ) {
        let mut active = mask;
        while active != 0 {
            let index = active.trailing_zeros() as usize;
            active &= active - 1;
            self.voice_filter_configs[index] = configs[index];
        }
    }

    pub(crate) fn push_voice_modulation_route(
        &mut self,
        source: u8,
        amount: f32,
        target: crate::modulation_target::TargetDescriptor,
    ) {
        self.voice_route_frame.push(source, amount, target);
    }

    pub(crate) fn push_voice_modulation_product_route(
        &mut self,
        source: u8,
        factor: u8,
        amount: f32,
        target: crate::modulation_target::TargetDescriptor,
    ) {
        self.voice_route_frame
            .push_product(source, factor, amount, target);
    }

    #[inline]
    fn invalidate_frame_control_cache(&mut self) {
        self.frame_control_valid = false;
    }

    fn unison_frame_control(
        &self,
        modulation: &[crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
        control: &mut UnisonFrameControl,
    ) {
        control.dynamic_position_mask = 0;
        control.active_mask = 0;
        control.spatial_mask = 0;
        control.spatial_shared_mask = 0;
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            let base = self.unison_settings[oscillator];
            let dynamic = modulation[oscillator];
            let amount_delta = dynamic.detune_amount;
            let range_delta = dynamic.detune_cents;
            let align_delta = dynamic.harmonic_align;
            let pitch_active = amount_delta.abs() > f32::EPSILON
                || range_delta.abs() > f32::EPSILON
                || align_delta.abs() > f32::EPSILON
                || dynamic.curve.abs() > ALIGNMENT_EPSILON;
            let spatial_active = dynamic.stereo.abs() > f32::EPSILON
                || dynamic.curve.abs() > ALIGNMENT_EPSILON
                || dynamic.stereo_x.abs() > f32::EPSILON
                || dynamic.stereo_y.abs() > f32::EPSILON
                || dynamic.weight.abs() > f32::EPSILON
                || dynamic.pan_center.abs() > f32::EPSILON
                || dynamic.pan_left.abs() > f32::EPSILON
                || dynamic.pan_right.abs() > f32::EPSILON
                || dynamic.pan_center_x.abs() > f32::EPSILON;
            let curve_active = dynamic.curve.abs() > ALIGNMENT_EPSILON;
            if (!pitch_active && !spatial_active) || base.voices <= 1 {
                continue;
            }
            let voices = usize::from(base.voices);
            if curve_active {
                let curve = base.curve + dynamic.curve;
                fill_unison_detune_positions(
                    &mut control.dynamic_detune_positions[oscillator],
                    base.voices,
                    curve,
                );
                control.dynamic_position_mask |= 1 << oscillator;
            }
            if spatial_active {
                control.spatial[oscillator] = dynamic;
                control.spatial_mask |= 1 << oscillator;
                let settings = base.modulated(dynamic);
                if stereo_square_weights(settings.stereo_alternate, settings.stereo_x)[2]
                    <= f32::EPSILON
                {
                    let template = &self.unison_templates[oscillator];
                    let simple_spatial = !curve_active
                        && dynamic.pan_center.abs() <= f32::EPSILON
                        && dynamic.pan_left.abs() <= f32::EPSILON
                        && dynamic.pan_right.abs() <= f32::EPSILON
                        && dynamic.pan_center_x.abs() <= f32::EPSILON;
                    control.spatial_gain[oscillator] = if simple_spatial {
                        build_spatial_from_components(
                            template,
                            settings,
                            &mut control.spatial_left[oscillator],
                            &mut control.spatial_right[oscillator],
                        )
                    } else if curve_active {
                        UnisonLayout::build_spatial_from_positions(
                            settings,
                            template.random_seed,
                            &control.dynamic_detune_positions[oscillator],
                            &mut control.spatial_left[oscillator],
                            &mut control.spatial_right[oscillator],
                        )
                    } else {
                        UnisonLayout::build_spatial_from_positions(
                            settings,
                            template.random_seed,
                            &template.detune_positions,
                            &mut control.spatial_left[oscillator],
                            &mut control.spatial_right[oscillator],
                        )
                    };
                    let render_voices = usize::from(template.render_voices);
                    control.spatial_left[oscillator][voices..render_voices].fill(0.0);
                    control.spatial_right[oscillator][voices..render_voices].fill(0.0);
                    control.spatial_shared_mask |= 1 << oscillator;
                }
            }
            let template = &self.unison_templates[oscillator];
            if !pitch_active {
                continue;
            }
            let effective_align = (base.harmonic_align + align_delta).clamp(0.0, 1.0);
            if effective_align <= ALIGNMENT_EPSILON {
                let effective_range = (base.detune_cents + range_delta).clamp(0.0, 4_800.0);
                let effective_amount = (base.detune_amount + amount_delta).clamp(0.0, 1.0);
                if curve_active {
                    let effective_cents = effective_range * effective_amount;
                    for index in 0..voices {
                        let raw_cents =
                            control.dynamic_detune_positions[oscillator][index] * effective_cents;
                        control.pitch_correction[oscillator][index] =
                            (raw_cents / 1_200.0).exp2() * template.ratio_reciprocals[index];
                    }
                } else {
                    let base_cents = base.detune_cents * base.detune_amount;
                    let scale = (effective_range * effective_amount - base_cents) / 1_200.0;
                    for (exponent, position) in control.exponents[..voices]
                        .iter_mut()
                        .zip(template.detune_positions[..voices].iter())
                    {
                        *exponent = *position * scale;
                    }
                    exp2_block(
                        &mut control.pitch_correction[oscillator][..voices],
                        &control.exponents[..voices],
                    );
                    let render_voices = usize::from(template.render_voices);
                    control.pitch_correction[oscillator][voices..render_voices].fill(0.0);
                }
            } else {
                let effective_range = (base.detune_cents + range_delta).clamp(0.0, 4_800.0);
                let effective_amount = (base.detune_amount + amount_delta).clamp(0.0, 1.0);
                let candidates = &self.harmonic_candidates[base.alignment_mode.index() as usize];
                let candidate_count = usize::from(
                    self.harmonic_candidate_counts[base.alignment_mode.index() as usize],
                );
                let cached_target = range_delta.abs() <= f32::EPSILON
                    && amount_delta.abs() <= f32::EPSILON
                    && !curve_active;
                let candidate_upper = if cached_target {
                    0
                } else {
                    harmonic_candidate_upper(
                        effective_range * effective_amount,
                        candidates,
                        candidate_count,
                    )
                };
                for index in 0..voices {
                    let detune_position = if curve_active {
                        control.dynamic_detune_positions[oscillator][index]
                    } else {
                        template.detune_positions[index]
                    };
                    let raw_cents = detune_position * effective_range * effective_amount;
                    let ratio = if effective_align <= ALIGNMENT_EPSILON {
                        (raw_cents / 1_200.0).exp2()
                    } else {
                        let target = if cached_target {
                            template.harmonic_targets[index]
                        } else {
                            nearest_harmonic_candidate_lattice(
                                raw_cents,
                                candidates,
                                candidate_upper,
                            )
                        };
                        let cents = raw_cents + effective_align * (target.cents - raw_cents);
                        if effective_align >= 1.0 {
                            target.ratio
                        } else {
                            (cents / 1_200.0).exp2()
                        }
                    };
                    control.pitch_correction[oscillator][index] =
                        ratio * template.ratio_reciprocals[index];
                }
                for correction in &mut control.pitch_correction[oscillator]
                    [voices..usize::from(template.render_voices)]
                {
                    *correction = 1.0;
                }
            }
            control.active_mask |= 1 << oscillator;
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        self.swarm_step =
            f64::from(self.unison_settings[0].swarm_rate) / f64::from(self.sample_rate);
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            self.secondary_swarm_step[secondary] =
                f64::from(self.unison_settings[secondary + 1].swarm_rate)
                    / f64::from(self.sample_rate);
        }
        for voice in &mut self.voices {
            voice.set_sample_rate(self.sample_rate);
        }
    }

    pub fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.reset();
        }
        self.age = 0;
        self.active_count = 0;
        self.sustain.fill(false);
        self.parameter_bend = 0.0;
        self.pitch_bend.fill(0.0);
        self.per_note_bend.fill(0.0);
        self.per_note_timbre.fill(None);
        self.timbre.fill(0.5);
        self.latest_stereo_seed.fill(0.5);
        self.swarm_time = 0.0;
        self.secondary_swarm_time.fill(0.0);
        for plan in self.resynth_playback.iter_mut() {
            plan.settle_after_reset();
        }
        // Telemetry identities are lifetime-monotonic. Resetting voice state
        // must not make a frozen pre-reset UI frame indistinguishable from a
        // newly published callback.
        self.mono_stack_len = 0;
    }

    pub fn configure_voice_mode(&mut self, mode: u8) {
        let mode = mode.clamp(0, POLYPHONY_U8);
        if self.voice_mode == mode {
            return;
        }
        for voice in &mut self.voices {
            if voice.active() {
                voice.release(true, self.sample_rate);
                voice.release_modulation(&self.voice_lfo_program);
            }
        }
        self.active_count = 0;
        self.settle_resynth_playback_if_idle();
        self.per_note_bend.fill(0.0);
        self.per_note_timbre.fill(None);
        self.mono_stack_len = 0;
        self.voice_mode = mode;
    }

    pub const fn set_glide_time(&mut self, seconds: f32) {
        self.glide_time = seconds.clamp(0.0, 5.0);
    }

    pub fn configure_oscillator_enabled(&mut self, enabled: [bool; LEGACY_OSCILLATOR_COUNT]) {
        let mask = enabled
            .into_iter()
            .enumerate()
            .fold(0, |mask, (oscillator, enabled)| {
                mask | (OscillatorMask::from(enabled) << oscillator)
            });
        if self.enabled_oscillator_mask == mask {
            return;
        }
        self.enabled_oscillator_mask = mask;
        for voice in &mut self.voices {
            voice.set_enabled_oscillator_mask(mask);
        }
    }

    /// Advance the monitor identities once for a completed host callback.
    /// The caller then publishes one frame per logical RESYNTH oscillator.
    pub(crate) fn begin_resynth_telemetry_block(&mut self, audio_frames: usize) -> (u64, u64) {
        self.resynth_publish_frame = self.resynth_publish_frame.wrapping_add(1);
        self.resynth_audio_frame = self.resynth_audio_frame.wrapping_add(audio_frames as u64);
        (self.resynth_publish_frame, self.resynth_audio_frame)
    }

    pub(crate) fn write_resynth_telemetry(
        &self,
        slot: usize,
        output: &mut crate::resynth_state::ResynthTelemetrySnapshot,
    ) {
        *output = crate::resynth_state::ResynthTelemetrySnapshot::default();
        let Some(plan) = self.resynth_playback.get(slot) else {
            return;
        };
        let slot_is_resynth = self
            .oscillator_bank
            .render()
            .entries()
            .iter()
            .find(|entry| usize::from(entry.slot) == slot)
            .is_some_and(|entry| {
                entry.current.engine.uses_sample_asset() || entry.target.engine.uses_sample_asset()
            });
        if !slot_is_resynth {
            return;
        }
        let from_identity = plan.from.publication_identity();
        let to_identity = plan.to.publication_identity();
        // Dynamic voice payload below is selected from the exact `to` layer;
        // never tag old scheduler state with the target publication identity.
        output.generation = to_identity.generation;
        output.from_generation = from_identity.generation;
        output.from_revision = from_identity.revision;
        output.to_generation = to_identity.generation;
        output.to_revision = to_identity.revision;
        output.transition_from_gain = plan.from_gain;
        output.transition_to_gain = plan.to_gain;
        output.transition_progress = plan.transition_progress();
        output.source_mix = plan.source_mix.clamp(0.0, 1.0);
        output.source_target = plan.source_target().clamp(0.0, 1.0);
        // SAFETY: telemetry is gathered inside the current audio callback;
        // the plan's live generations remain acknowledged until it completes.
        let source_len = unsafe { plan.to.artifact() }
            .map(|artifact| match &artifact.data {
                ProductionResynthArtifact::Sample(sample) => sample.samples.len(),
                ProductionResynthArtifact::Grain(grain) => grain.samples.len(),
                ProductionResynthArtifact::Rich(rich) => rich
                    .vocoder()
                    .map(|vocoder| vocoder.source_frames as usize)
                    .or_else(|| rich.sequence().map(|sequence| sequence.samples.len()))
                    .or_else(|| rich.slabs.as_deref().map(|slabs| slabs[0].len()))
                    .unwrap_or(1),
            })
            .unwrap_or(1);
        // Stable representative policy: the lowest-index active voice owns
        // monitor phase/lanes until it becomes inactive. Envelope crossings
        // must not jump the playhead between unrelated voice schedulers.
        if let Some(voice) = self.voices.iter().find(|voice| voice.active()) {
            let _ = voice.write_resynth_telemetry(slot, source_len, plan.to.generation(), output);
        }
    }

    /// Admit one coherent fixed publication set without partial plan mutation.
    /// The first pass is pure; the second is therefore infallible on the sole
    /// audio writer and performs no allocation, locking, or pointer ownership.
    pub(crate) fn try_retarget_resynth_batch(&mut self, update: &ResynthRtUpdate) -> bool {
        let voices_active = self.active_count != 0;
        for index in 0..MAX_OSCILLATORS {
            if update.changed_mask & (1_u32 << index) != 0
                && !self.resynth_playback[index].can_retarget(update.views[index], voices_active)
            {
                return false;
            }
        }
        let sample_rate = self.sample_rate;
        for index in 0..MAX_OSCILLATORS {
            if update.changed_mask & (1_u32 << index) == 0 {
                continue;
            }
            let accepted = self.resynth_playback[index].retarget(
                update.views[index],
                voices_active,
                sample_rate,
            );
            debug_assert!(accepted, "RESYNTH batch mutation violated its preflight");
        }
        true
    }

    pub(crate) fn set_resynth_grain_controls(
        &mut self,
        slot: usize,
        controls: crate::oscillators::ResynthControls,
    ) {
        if let Some(plan) = self.resynth_playback.get_mut(slot) {
            plan.grain_controls = controls;
            plan.grain_live = true;
        }
    }

    pub(crate) fn set_resynth_grain_curve(
        &mut self,
        slot: usize,
        curve: crate::wave_curve::WaveCurveRt,
    ) {
        if let Some(plan) = self.resynth_playback.get_mut(slot) {
            plan.grain_curve = curve;
        }
    }

    pub(crate) fn set_resynth_source_audition(&mut self, slot: usize, active: bool) -> bool {
        let Some(plan) = self.resynth_playback.get_mut(slot) else {
            return false;
        };
        if self.active_count == 0 {
            plan.snap_source_audition(active);
            true
        } else {
            plan.set_source_audition(active, self.sample_rate)
        }
    }

    pub(crate) fn resynth_plan_ack(&self, slot: usize) -> ResynthRtPlanAck {
        self.resynth_playback
            .get(slot)
            .map_or_else(ResynthRtPlanAck::default, |plan| ResynthRtPlanAck {
                live_generations: plan.live_generations(),
                accepted: plan.accepted_publication(),
            })
    }

    /// Algorithm of the replacement artifact currently owned by playback.
    #[must_use]
    pub(crate) fn sounding_resynth_algorithm(
        &self,
        slot: usize,
    ) -> Option<crate::oscillators::ResynthAlgorithm> {
        self.resynth_playback
            .get(slot)
            .and_then(ResynthPlaybackPlan::sounding_algorithm)
    }

    /// Installs a sounding plan in `slot` so tests can exercise the paths that
    /// are gated on a live RESYNTH publication.
    #[cfg(test)]
    pub(crate) fn install_test_resynth_plan(
        &mut self,
        slot: usize,
        algorithm: crate::oscillators::ResynthAlgorithm,
    ) {
        if let Some(plan) = self.resynth_playback.get_mut(slot) {
            plan.to =
                crate::resynth_state::publication::ResynthArtifactView::leaked_for_test(algorithm);
        }
    }

    pub(crate) fn has_active_resynth(&self) -> bool {
        self.oscillator_bank.render().entries().iter().any(|entry| {
            (entry.current.engine.uses_sample_asset() || entry.target.engine.uses_sample_asset())
                && self.resynth_playback[usize::from(entry.slot)].requires_render()
        })
    }

    pub(crate) fn resynth_transitioning(&self) -> bool {
        self.resynth_playback
            .iter()
            .any(ResynthPlaybackPlan::transitioning)
    }

    fn advance_resynth_playback(&mut self) {
        for plan in self.resynth_playback.iter_mut() {
            plan.advance();
        }
    }

    fn settle_resynth_playback_if_idle(&mut self) {
        if self.active_count == 0 {
            for plan in self.resynth_playback.iter_mut() {
                plan.settle_artifact_when_idle();
            }
        }
    }

    fn configure_envelope(&mut self, envelope: EnvelopeSettings) {
        if self.envelope == envelope {
            return;
        }
        self.envelope = envelope;
        for voice in &mut self.voices {
            voice.configure(envelope);
        }
    }

    pub(crate) fn configure_oscillators(
        &mut self,
        mut configs: [OscillatorDspConfig; MAX_OSCILLATORS],
    ) {
        for (slot, config) in configs.iter_mut().enumerate() {
            let resynth = config.engine.uses_sample_asset();
            if resynth {
                config.enabled &= self.resynth_playback[slot].requires_render();
            }
            if resynth
                && self.resynth_playback[slot].sounding_algorithm()
                    == Some(crate::oscillators::ResynthAlgorithm::Grain)
            {
                config.unison_voices = 1;
            }
            config.resynth_playback = if resynth {
                // SAFETY: the boxed plan slice is never resized or moved.
                unsafe { ResynthPlaybackPtr::new(std::ptr::from_ref(&self.resynth_playback[slot])) }
            } else {
                ResynthPlaybackPtr::NONE
            };
        }
        let newly_started = self.oscillator_bank.configure(configs, self.sample_rate);
        if self.active_count == 0 {
            self.oscillator_bank.snap_to_targets();
            return;
        }
        if newly_started != 0 {
            for voice in &mut self.voices {
                for slot in 0..MAX_OSCILLATORS {
                    if newly_started & (1 << slot) != 0 {
                        voice.oscillator_bank.seed_slot(
                            slot,
                            slot,
                            voice.note_seed,
                            self.oscillator_bank.render.entry(slot).current,
                        );
                    }
                }
            }
        }
    }

    pub(crate) fn generator_audio_modulation_active(&self) -> bool {
        self.generator_structural_route_frame.len != 0
            || self.generator_structural_route_frame.filter_len != 0
            || self.generator_structural_route_frame.aux_target_mask != 0
    }

    pub(crate) fn generator_aux_routes_active(&self) -> bool {
        self.generator_structural_route_frame.aux_routes_active()
    }

    pub(crate) fn generator_route_amount(&self, route_index: u8) -> Option<f32> {
        self.generator_structural_route_frame
            .route_amount(route_index)
    }

    pub(super) fn generator_pool_routes(
        &self,
        settings: VoiceSettings,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        filters: &[FilterCoefficients; MAX_FILTERS],
        fast_muted_sources: bool,
    ) -> Option<GeneratorStructuralRouteFrame> {
        let mut routes = self.generator_structural_route_frame;
        routes.retain_supported_filter_routes(filters);
        routes.enable_fast_muted_sources(
            settings.fast_audio_rate_modulation && fast_muted_sources,
            self.audible_oscillator_mask,
        );
        let mut modules = groups
            .iter()
            .take(group_count.min(MAX_OUTPUT_PAIRS))
            .flat_map(GeneratorRtGroup::modules);
        if modules.any(|module| matches!(module, crate::generators::GeneratorRtModule::Aux(_))) {
            return None;
        }
        Some(routes)
    }

    pub(crate) fn configure_filter_mask(&mut self, mask: u32) {
        for voice in &mut self.voices {
            voice.set_enabled_filter_mask(mask);
        }
    }

    pub(crate) fn configure_aux(&mut self, configs: [AuxConfig; MAX_AUX_MODULES]) {
        if self
            .aux_configs
            .iter()
            .zip(configs)
            .any(|(before, after)| before.source != after.source)
        {
            for voice in &mut self.voices {
                voice.reset_aux_taps();
            }
        }
        self.aux_configs = configs;
    }

    pub(crate) fn reset_generator_taps(&mut self) {
        for voice in &mut self.voices {
            voice.reset_aux_taps();
        }
    }

    pub(crate) fn reset_filter_states(&mut self) {
        for voice in &mut self.voices {
            voice.reset_filters();
        }
    }

    pub(crate) fn configure_output_groups(
        &mut self,
        envelopes: [EnvelopeSettings; MAX_OUTPUT_PAIRS],
        midi_channels: [u8; MAX_OUTPUT_PAIRS],
        count: usize,
        active_mask: u8,
        envelopes_enabled: bool,
    ) {
        let count = count.clamp(1, MAX_OUTPUT_PAIRS);
        let active_mask = active_mask & ((1_u16 << count) - 1) as u8;
        if self.output_group_envelopes == envelopes
            && self.output_group_midi_channels == midi_channels
            && usize::from(self.output_group_count) == count
            && self.output_group_active_mask == active_mask
            && self.output_group_envelopes_enabled == envelopes_enabled
        {
            return;
        }
        self.output_group_envelopes = envelopes;
        self.output_group_midi_channels = midi_channels.map(|channel| channel.min(16));
        self.output_group_count = count as u8;
        self.output_group_active_mask = active_mask;
        self.output_group_envelopes_enabled = envelopes_enabled;
        self.output_group_envelope_modulation_mask = 0;
        for voice in &mut self.voices {
            voice.configure_output_groups(
                self.output_group_envelopes,
                self.output_group_midi_channels,
                count,
                active_mask,
                envelopes_enabled,
            );
        }
        self.refresh_voice_count();
    }

    pub(crate) fn configure_audible_oscillators(&mut self, mask: u32) {
        self.audible_oscillator_mask = mask;
    }

    fn accepts_midi_channel(&self, channel: u8) -> bool {
        self.output_group_midi_channels[..usize::from(self.output_group_count)]
            .iter()
            .enumerate()
            .filter(|(index, _)| self.output_group_active_mask & (1 << index) != 0)
            .any(|(_, filter)| midi_channel_matches(*filter, channel.min(15)))
    }

    pub fn set_transpose(&mut self, semitones: f32) {
        let semitones = semitones.clamp(-60.0, 60.0);
        if self.transpose_semitones.to_bits() == semitones.to_bits() {
            return;
        }
        self.transpose_semitones = semitones;
        let global_bend = self.parameter_bend + self.pitch_bend[0];
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.active() {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(semitones + global_bend + member + self.per_note_bend[index]);
            }
        }
    }

    pub fn set_reference_tuning(&mut self, reference_hz: f32) {
        let reference_hz = reference_hz.clamp(1.0, 10_000.0);
        if self.reference_tuning_hz.to_bits() == reference_hz.to_bits() {
            return;
        }
        self.reference_tuning_hz = reference_hz;
        for voice in &mut self.voices {
            voice.set_reference_tuning(reference_hz);
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>) {
        let channel = channel.min(15);
        if !self.accepts_midi_channel(channel) {
            return;
        }
        if self.voice_mode < 2 {
            self.note_on_mono(note, velocity, channel, voice_id);
            return;
        }
        self.age = self.age.wrapping_add(1);
        self.set_latest_stereo_seeds(note_phase_seed(note, channel, voice_id, self.age));
        if let Some(index) = self
            .voices
            .iter()
            .position(|voice| voice.held && voice.matches(note, channel, voice_id))
        {
            self.per_note_bend[index] = 0.0;
            self.per_note_timbre[index] = None;
            self.voices[index].retrigger(velocity, voice_id, self.age);
            self.voices[index].trigger_modulation(&self.voice_lfo_program);
            self.voices[index].seed_oscillator_bank(self.oscillator_bank.render());
            self.voices[index]
                .set_pitch_bend(self.transpose_semitones + self.effective_pitch_bend(channel));
            self.voices[index].timbre = self.effective_timbre(channel);
            return;
        }

        let voice_limit = usize::from(self.voice_mode.clamp(2, POLYPHONY_U8));
        let index = self.voices[..voice_limit]
            .iter()
            .position(|voice| !voice.active())
            .unwrap_or_else(|| {
                self.voices[..voice_limit]
                    .iter()
                    .enumerate()
                    .min_by(|(_, left), (_, right)| {
                        left.envelope_level
                            .total_cmp(&right.envelope_level)
                            .then_with(|| left.age.cmp(&right.age))
                    })
                    .map_or(0, |(index, _)| index)
            });
        let was_active = self.voices[index].active();
        self.per_note_bend[index] = 0.0;
        self.per_note_timbre[index] = None;
        self.prepare_voice_unison(index);
        self.voices[index].start(note, velocity, channel, voice_id, self.age);
        self.voices[index].trigger_modulation(&self.voice_lfo_program);
        self.voices[index].seed_oscillator_bank(self.oscillator_bank.render());
        self.voices[index]
            .set_pitch_bend(self.transpose_semitones + self.effective_pitch_bend(channel));
        self.voices[index].timbre = self.effective_timbre(channel);
        if !was_active {
            self.active_count += 1;
        }
    }

    fn note_on_mono(&mut self, note: u8, velocity: f32, channel: u8, voice_id: Option<i32>) {
        let channel = channel.min(15);
        self.remove_mono_note(note, channel, voice_id);
        let connect_legato = self.mono_stack_len != 0;
        if usize::from(self.mono_stack_len) == POLYPHONY {
            self.mono_stack.copy_within(1..POLYPHONY, 0);
            self.mono_stack_len -= 1;
        }
        self.mono_stack[usize::from(self.mono_stack_len)] = HeldNote {
            note,
            velocity,
            channel,
            voice_id,
            per_note_bend: 0.0,
            per_note_timbre: None,
        };
        self.mono_stack_len += 1;

        self.age = self.age.wrapping_add(1);
        let next_seed = note_phase_seed(note, channel, voice_id, self.age);
        let pitch_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        let timbre = self.effective_timbre(channel);
        let oscillator_bank = self.oscillator_bank.render();
        let voice = &mut self.voices[0];
        self.per_note_bend[0] = 0.0;
        self.per_note_timbre[0] = None;
        if self.voice_mode == 1 && connect_legato {
            voice.legato_to(note, velocity, channel, voice_id, self.age, self.glide_time);
            voice.modulation.retarget_note(note);
        } else {
            self.latest_stereo_seed =
                std::array::from_fn(|oscillator| oscillator_stereo_seed(next_seed, oscillator));
            for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                if oscillator == 0 {
                    voice.configure_unison(self.unison_settings[oscillator]);
                } else {
                    voice.configure_secondary_unison(oscillator, self.unison_settings[oscillator]);
                }
            }
            voice.start(note, velocity, channel, voice_id, self.age);
            voice.trigger_modulation(&self.voice_lfo_program);
            voice.seed_oscillator_bank(oscillator_bank);
        }
        voice.set_pitch_bend(pitch_bend);
        voice.timbre = timbre;
        self.active_count = 1;
    }

    pub fn note_off(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        if self.voice_mode < 2 {
            self.note_off_mono(note, channel, voice_id);
            return;
        }
        let channel = channel.min(15);
        let finished = if let Some(voice) = self
            .voices
            .iter_mut()
            .filter(|voice| voice.matches(note, channel, voice_id) && voice.held)
            .max_by_key(|voice| voice.age)
        {
            voice.sustained = self.sustain[channel as usize];
            voice.release(false, self.sample_rate);
            voice.release_modulation(&self.voice_lfo_program);
            !voice.active()
        } else {
            false
        };
        if finished {
            self.active_count -= 1;
            if self.active_count == 0 {
                settle_resynth_plans(&mut self.resynth_playback);
            }
        }
    }

    fn note_off_mono(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        let channel = channel.min(15);
        let was_current = self.voices[0].matches(note, channel, voice_id);
        self.remove_mono_note(note, channel, voice_id);
        if !was_current {
            return;
        }
        if self.mono_stack_len != 0 {
            let held = self.mono_stack[usize::from(self.mono_stack_len - 1)];
            self.age = self.age.wrapping_add(1);
            let next_seed = note_phase_seed(held.note, held.channel, held.voice_id, self.age);
            let pitch_bend = self.transpose_semitones
                + self.effective_pitch_bend(held.channel)
                + held.per_note_bend;
            let timbre = self.effective_timbre(held.channel);
            let oscillator_bank = self.oscillator_bank.render();
            let voice = &mut self.voices[0];
            if self.voice_mode == 1 {
                voice.legato_to(
                    held.note,
                    held.velocity,
                    held.channel,
                    held.voice_id,
                    self.age,
                    self.glide_time,
                );
                voice.modulation.retarget_note(held.note);
            } else {
                self.latest_stereo_seed =
                    std::array::from_fn(|oscillator| oscillator_stereo_seed(next_seed, oscillator));
                voice.start(
                    held.note,
                    held.velocity,
                    held.channel,
                    held.voice_id,
                    self.age,
                );
                voice.trigger_modulation(&self.voice_lfo_program);
                voice.seed_oscillator_bank(oscillator_bank);
            }
            voice.set_pitch_bend(pitch_bend);
            self.per_note_bend[0] = held.per_note_bend;
            self.per_note_timbre[0] = held.per_note_timbre;
            voice.timbre = held.per_note_timbre.unwrap_or(timbre);
            return;
        }
        let voice = &mut self.voices[0];
        voice.sustained = self.sustain[channel as usize];
        voice.release(false, self.sample_rate);
        voice.release_modulation(&self.voice_lfo_program);
        if !voice.active() {
            self.active_count = 0;
            self.settle_resynth_playback_if_idle();
        }
    }

    fn remove_mono_note(&mut self, note: u8, channel: u8, voice_id: Option<i32>) {
        let len = usize::from(self.mono_stack_len);
        if let Some(index) = self.mono_stack[..len].iter().position(|held| {
            held.note == note
                && held.channel == channel
                && voice_id.is_none_or(|id| held.voice_id == Some(id))
        }) {
            self.mono_stack.copy_within(index + 1..len, index);
            self.mono_stack_len -= 1;
        }
    }

    pub fn all_notes_off(&mut self, channel: u8) {
        let channel = channel.min(15);
        if self.voice_mode < 2 {
            self.clear_mono_channel(channel);
        }
        let mut finished = 0_u8;
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() && voice.held {
                voice.sustained = self.sustain[voice.channel as usize];
                voice.release(false, self.sample_rate);
                voice.release_modulation(&self.voice_lfo_program);
                finished += u8::from(!voice.active());
            }
        }
        self.active_count -= finished;
        self.settle_resynth_playback_if_idle();
    }

    pub fn all_sound_off(&mut self, channel: u8) {
        let channel = channel.min(15);
        if self.voice_mode < 2 {
            self.clear_mono_channel(channel);
        }
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                voice.sustained = false;
                voice.release(true, self.sample_rate);
                voice.release_modulation(&self.voice_lfo_program);
            }
        }
        self.refresh_voice_count();
    }

    pub fn reset_controllers(&mut self, channel: u8) {
        let channel = channel.min(15);
        self.sustain(channel, false);
        self.pitch_bend(channel, 0.0, 2.0);
        self.timbre(channel, 0.5);
    }

    pub fn pressure(&mut self, note: u8, channel: u8, voice_id: Option<i32>, pressure: f32) {
        for voice in &mut self.voices {
            if voice.matches(note, channel, voice_id) {
                voice.pressure = pressure.clamp(0.0, 1.0);
            }
        }
    }

    pub fn channel_pressure(&mut self, channel: u8, pressure: f32) {
        let channel = channel.min(15);
        for voice in &mut self.voices {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                voice.pressure = pressure.clamp(0.0, 1.0);
            }
        }
    }

    pub fn pitch_bend(&mut self, channel: u8, bipolar: f32, mpe_range: f32) {
        self.pitch_bend_asymmetric(channel, bipolar, mpe_range, mpe_range);
    }

    pub fn pitch_bend_asymmetric(
        &mut self,
        channel: u8,
        bipolar: f32,
        down_range: f32,
        up_range: f32,
    ) {
        let channel = channel.min(15);
        let bipolar = bipolar.clamp(-1.0, 1.0);
        let range = (if bipolar < 0.0 { down_range } else { up_range }).clamp(1.0, 96.0);
        let semitones = bipolar * range;
        if self.pitch_bend[channel as usize].to_bits() == semitones.to_bits() {
            return;
        }
        self.pitch_bend[channel as usize] = semitones;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if channel == 0 || voice.channel == channel {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(
                    self.transpose_semitones
                        + self.parameter_bend
                        + self.pitch_bend[0]
                        + member
                        + self.per_note_bend[index],
                );
            }
        }
    }

    pub fn parameter_pitch_bend(&mut self, bipolar: f32, range: f32) {
        self.parameter_pitch_bend_asymmetric(bipolar, range, range);
    }

    pub fn parameter_pitch_bend_asymmetric(
        &mut self,
        bipolar: f32,
        down_range: f32,
        up_range: f32,
    ) {
        let bipolar = bipolar.clamp(-1.0, 1.0);
        let range = (if bipolar < 0.0 { down_range } else { up_range }).clamp(1.0, 96.0);
        let semitones = bipolar * range;
        if self.parameter_bend.to_bits() == semitones.to_bits() {
            return;
        }
        self.parameter_bend = semitones;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.active() {
                let member = if voice.channel == 0 {
                    0.0
                } else {
                    self.pitch_bend[voice.channel as usize]
                };
                voice.set_pitch_bend(
                    self.transpose_semitones
                        + self.parameter_bend
                        + self.pitch_bend[0]
                        + member
                        + self.per_note_bend[index],
                );
            }
        }
    }

    pub fn per_note_pitch_bend(&mut self, note: u8, channel: u8, semitones: f32) {
        let channel = channel.min(15);
        let channel_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        let semitones = semitones.clamp(-96.0, 96.0);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_bend[index] = semitones;
                voice.set_pitch_bend(channel_bend + semitones);
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_bend = semitones;
            }
        }
    }

    pub fn per_note_timbre(&mut self, note: u8, channel: u8, value: f32) {
        let channel = channel.min(15);
        let value = value.clamp(0.0, 1.0);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_timbre[index] = Some(value);
                voice.timbre = value;
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_timbre = Some(value);
            }
        }
    }

    pub fn reset_per_note_controllers(&mut self, note: u8, channel: u8) {
        let channel = channel.min(15);
        let fallback = self.effective_timbre(channel);
        let pitch_bend = self.transpose_semitones + self.effective_pitch_bend(channel);
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if voice.matches(note, channel, None) {
                self.per_note_bend[index] = 0.0;
                self.per_note_timbre[index] = None;
                voice.set_pitch_bend(pitch_bend);
                voice.timbre = fallback;
            }
        }
        for held in &mut self.mono_stack[..usize::from(self.mono_stack_len)] {
            if held.note == note && held.channel == channel {
                held.per_note_bend = 0.0;
                held.per_note_timbre = None;
            }
        }
    }

    pub fn timbre(&mut self, channel: u8, value: f32) {
        let channel = channel.min(15);
        let value = value.clamp(0.0, 1.0);
        self.timbre[channel as usize] = value;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if (channel == 0 || voice.channel == channel) && voice.active() {
                let member = if voice.channel == 0 {
                    0.5
                } else {
                    self.timbre[voice.channel as usize]
                };
                voice.timbre = self.per_note_timbre[index]
                    .unwrap_or_else(|| (self.timbre[0] + member - 0.5).clamp(0.0, 1.0));
            }
        }
    }

    pub fn configure_unison(&mut self, settings: UnisonSettings) {
        self.schedule_unison_configuration(0, settings);
    }

    pub fn configure_secondary_unison(&mut self, oscillator: usize, settings: UnisonSettings) {
        self.schedule_unison_configuration(oscillator, settings);
    }

    pub fn configure_unison_motion(&mut self, oscillator: usize, settings: UnisonSettings) {
        let current = self.unison_settings[oscillator];
        let motion_changed = current.phase_random.to_bits() != settings.phase_random.to_bits()
            || current.swarm_amount.to_bits() != settings.swarm_amount.to_bits()
            || current.swarm_rate.to_bits() != settings.swarm_rate.to_bits()
            || current.swarm_mode != settings.swarm_mode;
        if !motion_changed {
            return;
        }
        self.unison_settings[oscillator] = current.with_motion(
            settings.phase_random,
            settings.swarm_amount,
            settings.swarm_rate,
        );
        self.unison_settings[oscillator].swarm_mode = settings.swarm_mode;
        self.unison_templates[oscillator].configure_motion(self.unison_settings[oscillator]);
        if oscillator == 0 {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_unison_motion(self.unison_settings[oscillator]);
            }
            self.swarm_step = f64::from(self.unison_settings[oscillator].swarm_rate)
                / f64::from(self.sample_rate);
        } else {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_secondary_unison_motion(
                    oscillator,
                    self.unison_settings[oscillator],
                );
            }
            self.secondary_swarm_step[oscillator - 1] =
                f64::from(self.unison_settings[oscillator].swarm_rate)
                    / f64::from(self.sample_rate);
        }
    }

    fn schedule_unison_configuration(&mut self, oscillator: usize, settings: UnisonSettings) {
        if self.unison_settings[oscillator] != settings {
            self.apply_unison_configuration(oscillator, settings);
        }
    }

    fn apply_unison_configuration(&mut self, oscillator: usize, settings: UnisonSettings) {
        let previous = self.unison_settings[oscillator];
        self.invalidate_frame_control_cache();
        let tuning_changed = previous.voices != settings.voices
            || previous.detune_cents.to_bits() != settings.detune_cents.to_bits()
            || previous.curve.to_bits() != settings.curve.to_bits()
            || previous.detune_amount.to_bits() != settings.detune_amount.to_bits()
            || previous.harmonic_align.to_bits() != settings.harmonic_align.to_bits()
            || previous.alignment_mode != settings.alignment_mode;
        self.unison_settings[oscillator] = settings;
        self.unison_templates[oscillator].configure(settings, self.sample_rate, false);
        if tuning_changed {
            self.refresh_harmonic_targets(oscillator);
        }
        let prepared = &self.unison_templates[oscillator];
        if oscillator == 0 {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_unison_with_prepared(settings, Some(prepared));
            }
            self.swarm_step = f64::from(settings.swarm_rate) / f64::from(self.sample_rate);
        } else {
            for voice in self.voices.iter_mut().filter(|voice| voice.active()) {
                voice.configure_secondary_unison_with_prepared(
                    oscillator,
                    settings,
                    Some(prepared),
                );
            }
            self.secondary_swarm_step[oscillator - 1] =
                f64::from(settings.swarm_rate) / f64::from(self.sample_rate);
        }
    }

    fn refresh_harmonic_targets(&mut self, oscillator: usize) {
        let template = &self.unison_templates[oscillator];
        let settings = template.settings;
        let candidates = self.harmonic_candidates[settings.alignment_mode.index() as usize];
        let candidate_count =
            self.harmonic_candidate_counts[settings.alignment_mode.index() as usize];
        let candidate_upper = harmonic_candidate_upper(
            settings.detune_cents * settings.detune_amount,
            &candidates,
            usize::from(candidate_count),
        );
        let positions = template.detune_positions;
        let targets = &mut self.unison_templates[oscillator].harmonic_targets;
        targets.fill(EMPTY_ALIGNMENT_CANDIDATE);
        for index in 0..usize::from(settings.voices) {
            let raw_cents = positions[index] * settings.detune_cents * settings.detune_amount;
            targets[index] =
                nearest_harmonic_candidate_lattice(raw_cents, &candidates, candidate_upper);
        }
    }

    fn prepare_voice_unison(&mut self, index: usize) {
        let voice = &mut self.voices[index];
        voice.unison.copy_prepared_from(&self.unison_templates[0]);
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            voice.secondary_unison[secondary]
                .copy_prepared_from(&self.unison_templates[secondary + 1]);
        }
        voice.phase_steps_dirty = true;
        voice.secondary_phase_steps_dirty.fill(true);
    }

    pub fn configure_phase_warp_modes(&mut self, modes: [PhaseWarpMode; LEGACY_OSCILLATOR_COUNT]) {
        for (oscillator, mode) in modes.into_iter().enumerate() {
            if self.phase_warp_mode[oscillator] != mode {
                self.phase_warp_mode[oscillator] = mode;
            }
        }
    }

    pub fn sustain(&mut self, channel: u8, enabled: bool) {
        let channel = channel.min(15);
        if channel == 0 {
            self.sustain.fill(enabled);
        } else {
            self.sustain[channel as usize] = enabled;
        }
        if !enabled {
            let mut finished = 0_u8;
            for voice in &mut self.voices {
                if (channel == 0 || voice.channel == channel) && voice.sustained && !voice.held {
                    voice.sustained = false;
                    voice.release(false, self.sample_rate);
                    voice.release_modulation(&self.voice_lfo_program);
                    finished += u8::from(!voice.active());
                }
            }
            self.active_count -= finished;
            self.settle_resynth_playback_if_idle();
        }
    }

    pub(crate) fn unison_layouts_steady(&self) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(VaVoice::unison_transitions_steady)
    }

    fn apply_phase_warp_modes(&self, settings: &mut VoiceSettings) {
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            settings.oscillators[oscillator].phase_warp.mode = self.phase_warp_mode[oscillator];
        }
    }

    pub(super) fn apply_oscillator_state(&self, mut settings: VoiceSettings) -> VoiceSettings {
        self.apply_phase_warp_modes(&mut settings);
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            let enabled = self.enabled_oscillator_mask & (1 << oscillator) != 0;
            settings.oscillators[oscillator].enabled = enabled;
        }
        settings
    }

    pub fn render(&mut self, settings: VoiceSettings, envelope: EnvelopeSettings) -> (f32, f32) {
        self.render_with_unison_control::<false>(
            settings,
            envelope,
            &UnisonFrameControl::NEUTRAL,
            &StructuralOscillatorFrameControl::NEUTRAL,
        )
    }

    pub(crate) fn render_neutral(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> (f32, f32) {
        self.render_neutral_with_structural_frame(
            settings,
            envelope,
            &StructuralOscillatorFrameControl::NEUTRAL,
        )
    }

    pub(crate) fn render_neutral_with_structural_frame(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        structural_control: &StructuralOscillatorFrameControl,
    ) -> (f32, f32) {
        self.invalidate_frame_control_cache();
        self.render_with_unison_control::<false>(
            settings,
            envelope,
            &UnisonFrameControl::NEUTRAL,
            structural_control,
        )
    }

    pub(crate) fn render_grouped_neutral(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        groups: &[GeneratorRtGroup],
        filters: &[FilterCoefficients; MAX_FILTERS],
        ordered_filter_render: bool,
    ) -> [(f32, f32); MAX_OUTPUT_PAIRS] {
        self.render_grouped_neutral_with_structural_frame(
            settings,
            envelope,
            &StructuralOscillatorFrameControl::NEUTRAL,
            oscillator_groups,
            group_count,
            groups,
            filters,
            ordered_filter_render,
        )
    }

    pub(crate) fn render_grouped_neutral_with_structural_frame(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        structural_control: &StructuralOscillatorFrameControl,
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        groups: &[GeneratorRtGroup],
        filters: &[FilterCoefficients; MAX_FILTERS],
        ordered_filter_render: bool,
    ) -> [(f32, f32); MAX_OUTPUT_PAIRS] {
        self.restore_output_group_envelopes();
        self.invalidate_frame_control_cache();
        self.render_grouped_with_unison_control::<false>(
            settings,
            envelope,
            &UnisonFrameControl::NEUTRAL,
            structural_control,
            oscillator_groups,
            group_count,
            groups,
            filters,
            ordered_filter_render,
        )
    }

    pub(crate) fn render_with_modulation_and_structural_frame(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        modulation: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
        structural_control: &StructuralOscillatorFrameControl,
    ) -> (f32, f32) {
        if self.active_count == 0 {
            return (0.0, 0.0);
        }
        if self.voice_lfo_program.active() {
            return self.render_with_voice_modulation(
                settings,
                envelope,
                modulation,
                structural_control,
            );
        }
        if modulation
            .iter()
            .any(crate::modulators::lfo::UnisonModulation::frame_active)
        {
            let mut frame_control = self
                .frame_control_cache
                .take()
                .expect("unison frame control cache must be initialized");
            if !self.frame_control_valid || self.frame_control_modulation != modulation {
                self.unison_frame_control(&modulation, &mut frame_control);
                self.frame_control_modulation = modulation;
                self.frame_control_valid = true;
            }
            let output = self.render_with_unison_control::<true>(
                settings,
                envelope,
                &frame_control,
                structural_control,
            );
            self.frame_control_cache = Some(frame_control);
            output
        } else {
            self.invalidate_frame_control_cache();
            self.render_with_unison_control::<false>(
                settings,
                envelope,
                &UnisonFrameControl::NEUTRAL,
                structural_control,
            )
        }
    }

    pub(crate) fn render_grouped_with_modulation_and_structural_frame(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        modulation: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
        structural_control: &StructuralOscillatorFrameControl,
        structural: &crate::StructuralModulationFrame,
        group_outputs: &[GroupOutput],
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        groups: &[GeneratorRtGroup],
        filters: &[FilterCoefficients; MAX_FILTERS],
        ordered_filter_render: bool,
    ) -> [(f32, f32); MAX_OUTPUT_PAIRS] {
        if self.active_count == 0 {
            self.restore_output_group_envelopes();
            return [(0.0, 0.0); MAX_OUTPUT_PAIRS];
        }
        if self.voice_lfo_program.active() {
            let group_envelope_mask = structural.group_envelope_mask
                | self.voice_structural_route_frame.group_envelope_mask();
            self.retain_output_group_envelope_modulation(group_envelope_mask);
            self.output_group_envelope_modulation_mask = group_envelope_mask;
            return self.render_grouped_with_voice_modulation(
                settings,
                envelope,
                modulation,
                structural_control,
                structural,
                group_outputs,
                oscillator_groups,
                group_count,
                groups,
                filters,
                ordered_filter_render,
            );
        }
        self.configure_shared_output_group_envelopes(structural);
        if modulation
            .iter()
            .any(crate::modulators::lfo::UnisonModulation::frame_active)
        {
            let mut frame_control = self
                .frame_control_cache
                .take()
                .expect("unison frame control cache must be initialized");
            if !self.frame_control_valid || self.frame_control_modulation != modulation {
                self.unison_frame_control(&modulation, &mut frame_control);
                self.frame_control_modulation = modulation;
                self.frame_control_valid = true;
            }
            let output = self.render_grouped_with_unison_control::<true>(
                settings,
                envelope,
                &frame_control,
                structural_control,
                oscillator_groups,
                group_count,
                groups,
                filters,
                ordered_filter_render,
            );
            self.frame_control_cache = Some(frame_control);
            output
        } else {
            self.invalidate_frame_control_cache();
            self.render_grouped_with_unison_control::<false>(
                settings,
                envelope,
                &UnisonFrameControl::NEUTRAL,
                structural_control,
                oscillator_groups,
                group_count,
                groups,
                filters,
                ordered_filter_render,
            )
        }
    }

    fn render_with_voice_modulation(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        global_unison: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
        structural_control: &StructuralOscillatorFrameControl,
    ) -> (f32, f32) {
        let settings = self.apply_oscillator_state(settings);
        self.advance_shared_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let mut frame_control = self
            .frame_control_cache
            .take()
            .expect("unison frame control cache must be initialized");
        let mut output = (0.0_f32, 0.0_f32);
        let mut remaining = self.active_count;
        let legacy_modulation_active = self.voice_route_frame.active();
        let mut voice_structural = crate::StructuralModulationFrame::default();
        for index in 0..self.voices.len() {
            if !self.voices[index].active() {
                continue;
            }
            let voice_modulation = {
                let voice = &mut self.voices[index];
                let values = voice.modulation.next(&self.voice_lfo_program);
                let modulation = self.voice_route_frame.evaluate_values(values);
                self.voice_structural_route_frame
                    .evaluate(values, &mut voice_structural);
                modulation
            };
            let mut voice_settings = settings;
            let voice_envelope = if legacy_modulation_active {
                apply_voice_settings(&mut voice_settings, voice_modulation);
                modulated_voice_envelope(envelope, voice_modulation.global)
            } else {
                envelope
            };
            let combined_unison = std::array::from_fn(|oscillator| {
                add_unison_modulation(
                    global_unison[oscillator],
                    voice_modulation.unison[oscillator],
                )
            });
            let unison_active = combined_unison
                .iter()
                .any(crate::modulators::lfo::UnisonModulation::frame_active);
            if unison_active {
                self.unison_frame_control(&combined_unison, &mut frame_control);
            }
            let voice_structural_control = merge_voice_structural_control(
                structural_control,
                &voice_structural,
                &self.oscillator_bank,
            );
            let voice = &mut self.voices[index];
            voice.configure(voice_envelope);
            if unison_active {
                apply_voice_unison_motion(voice, &self.unison_settings, &combined_unison);
            }
            set_voice_swarm_clocks(
                voice,
                voice_settings,
                self.swarm_time,
                self.secondary_swarm_time,
            );
            let (voice_left, voice_right) = if unison_active {
                voice.render_controlled::<true>(
                    voice_settings,
                    self.sample_rate,
                    false,
                    &frame_control,
                )
            } else {
                voice.render_controlled::<false>(
                    voice_settings,
                    self.sample_rate,
                    false,
                    &UnisonFrameControl::NEUTRAL,
                )
            };
            let (bank_left, bank_right) = voice.render_oscillator_bank(
                oscillator_bank,
                voice_settings,
                self.sample_rate,
                &voice_structural_control,
            );
            let gain = if legacy_modulation_active {
                voice_output_gain(voice_modulation.global.output_db)
            } else {
                1.0
            };
            output.0 += (voice_left + bank_left) * gain;
            output.1 += (voice_right + bank_right) * gain;
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }
        self.frame_control_cache = Some(frame_control);
        self.frame_control_valid = false;
        (output.0 * MASTER_HEADROOM, output.1 * MASTER_HEADROOM)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_grouped_with_voice_modulation(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        global_unison: [crate::modulators::lfo::UnisonModulation; LEGACY_OSCILLATOR_COUNT],
        structural_control: &StructuralOscillatorFrameControl,
        structural: &crate::StructuralModulationFrame,
        group_outputs: &[GroupOutput],
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        groups: &[GeneratorRtGroup],
        filters: &[FilterCoefficients; MAX_FILTERS],
        ordered_filter_render: bool,
    ) -> [(f32, f32); MAX_OUTPUT_PAIRS] {
        let settings = self.apply_oscillator_state(settings);
        self.advance_shared_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let mut frame_control = self
            .frame_control_cache
            .take()
            .expect("unison frame control cache must be initialized");
        let mut stems = [(0.0_f32, 0.0_f32); MAX_OUTPUT_PAIRS];
        let generator_routes = self.generator_structural_route_frame;
        let aux_configs = self.aux_configs;
        let mut remaining = self.active_count;
        let legacy_modulation_active = self.voice_route_frame.active();
        let voice_group_mask = self.voice_structural_route_frame.group_gain_pan_mask();
        let base_group_envelopes = self.output_group_envelopes;
        let mut voice_structural = crate::StructuralModulationFrame::default();
        for index in 0..self.voices.len() {
            if !self.voices[index].active() {
                continue;
            }
            let voice_modulation = {
                let voice = &mut self.voices[index];
                let values = voice.modulation.next(&self.voice_lfo_program);
                let modulation = self.voice_route_frame.evaluate_values(values);
                self.voice_structural_route_frame
                    .evaluate(values, &mut voice_structural);
                modulation
            };
            let mut voice_settings = settings;
            let voice_envelope = if legacy_modulation_active {
                apply_voice_settings(&mut voice_settings, voice_modulation);
                modulated_voice_envelope(envelope, voice_modulation.global)
            } else {
                envelope
            };
            let combined_unison = std::array::from_fn(|oscillator| {
                add_unison_modulation(
                    global_unison[oscillator],
                    voice_modulation.unison[oscillator],
                )
            });
            let unison_active = combined_unison
                .iter()
                .any(crate::modulators::lfo::UnisonModulation::frame_active);
            if unison_active {
                self.unison_frame_control(&combined_unison, &mut frame_control);
            }
            let voice_structural_control = merge_voice_structural_control(
                structural_control,
                &voice_structural,
                &self.oscillator_bank,
            );
            let voice_filters = merge_voice_filter_coefficients(
                &self.voice_filter_configs,
                filters,
                &voice_structural,
                self.sample_rate,
            );
            let ratio_bands = structural_ratio_bands(groups, group_count, &voice_filters);
            let voice = &mut self.voices[index];
            let mut group_envelope_mask =
                structural.group_envelope_mask | voice_structural.group_envelope_mask;
            while group_envelope_mask != 0 {
                let group = group_envelope_mask.trailing_zeros() as usize;
                group_envelope_mask &= group_envelope_mask - 1;
                if group < group_count {
                    let voice_delta = if voice_structural.group_mask & (1 << group) != 0 {
                        voice_structural.groups[group]
                    } else {
                        crate::StructuralGroupDelta::default()
                    };
                    let shared_delta = if structural.group_mask & (1 << group) != 0 {
                        structural.groups[group]
                    } else {
                        crate::StructuralGroupDelta::default()
                    };
                    voice.configure_output_group_envelope(
                        group,
                        modulated_group_envelope(
                            base_group_envelopes[group],
                            voice_delta,
                            shared_delta,
                        ),
                    );
                }
            }
            voice.configure(voice_envelope);
            if unison_active {
                apply_voice_unison_motion(voice, &self.unison_settings, &combined_unison);
            }
            set_voice_swarm_clocks(
                voice,
                voice_settings,
                self.swarm_time,
                self.secondary_swarm_time,
            );
            let mut voice_stems = [(0.0_f32, 0.0_f32); MAX_OUTPUT_PAIRS];
            if unison_active {
                voice.render_controlled_grouped::<true>(
                    voice_settings,
                    self.sample_rate,
                    &frame_control,
                    &mut voice_stems,
                    oscillator_groups,
                    group_count,
                );
            } else {
                voice.render_controlled_grouped::<false>(
                    voice_settings,
                    self.sample_rate,
                    &UnisonFrameControl::NEUTRAL,
                    &mut voice_stems,
                    oscillator_groups,
                    group_count,
                );
            }
            if ordered_filter_render {
                voice.render_ordered_oscillator_groups(
                    oscillator_bank,
                    voice_settings,
                    self.sample_rate,
                    &voice_structural_control,
                    &mut voice_stems,
                    groups,
                    group_count,
                    &voice_filters,
                    &ratio_bands,
                    &aux_configs,
                    &generator_routes,
                    None,
                );
            } else {
                voice.render_oscillator_bank_grouped(
                    oscillator_bank,
                    voice_settings,
                    self.sample_rate,
                    &voice_structural_control,
                    &mut voice_stems,
                    oscillator_groups,
                    group_count,
                );
            }
            let gain = if legacy_modulation_active {
                voice_output_gain(voice_modulation.global.output_db)
            } else {
                1.0
            };
            for group in 0..group_count {
                if voice_group_mask & (1 << group) != 0 {
                    let voice_delta = voice_structural.groups[group];
                    let shared_delta = if structural.group_mask & (1 << group) != 0 {
                        structural.groups[group]
                    } else {
                        crate::StructuralGroupDelta::default()
                    };
                    apply_group_gain_pan(
                        &mut voice_stems[group],
                        group_outputs[group],
                        voice_delta,
                        shared_delta,
                    );
                }
                stems[group].0 += voice_stems[group].0 * gain;
                stems[group].1 += voice_stems[group].1 * gain;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
            remaining -= 1;
            if remaining == 0 {
                break;
            }
        }
        self.frame_control_cache = Some(frame_control);
        self.frame_control_valid = false;
        for stem in &mut stems {
            stem.0 *= MASTER_HEADROOM;
            stem.1 *= MASTER_HEADROOM;
        }
        stems
    }

    fn advance_shared_oscillator_state(&mut self, settings: VoiceSettings) {
        if settings.oscillator(0).enabled {
            self.swarm_time = wrap_swarm_time(self.swarm_time + self.swarm_step);
        }
        for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
            if settings.oscillator(secondary + 1).enabled {
                self.secondary_swarm_time[secondary] = wrap_swarm_time(
                    self.secondary_swarm_time[secondary] + self.secondary_swarm_step[secondary],
                );
            }
        }
        self.oscillator_bank.advance(self.sample_rate);
        self.advance_resynth_playback();
    }

    fn render_with_unison_control<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        unison_control: &UnisonFrameControl,
        structural_control: &StructuralOscillatorFrameControl,
    ) -> (f32, f32) {
        if self.active_count == 0 {
            return (0.0, 0.0);
        }

        self.configure_envelope(envelope);

        let settings = self.apply_oscillator_state(settings);
        let mut left = 0.0;
        let mut right = 0.0;
        self.advance_shared_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            set_voice_swarm_clocks(voice, settings, self.swarm_time, self.secondary_swarm_time);
            let (voice_left, voice_right) = voice.render_controlled::<DYNAMIC_UNISON>(
                settings,
                self.sample_rate,
                false,
                unison_control,
            );
            let (bank_left, bank_right) = voice.render_oscillator_bank(
                oscillator_bank,
                settings,
                self.sample_rate,
                structural_control,
            );
            left += voice_left + bank_left;
            right += voice_right + bank_right;
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        (left * MASTER_HEADROOM, right * MASTER_HEADROOM)
    }

    fn render_grouped_with_unison_control<const DYNAMIC_UNISON: bool>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        unison_control: &UnisonFrameControl,
        structural_control: &StructuralOscillatorFrameControl,
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
        groups: &[GeneratorRtGroup],
        filters: &[FilterCoefficients; MAX_FILTERS],
        ordered_filter_render: bool,
    ) -> [(f32, f32); MAX_OUTPUT_PAIRS] {
        let mut stems = [(0.0, 0.0); MAX_OUTPUT_PAIRS];
        if self.active_count == 0 {
            return stems;
        }
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        self.configure_envelope(envelope);

        let settings = self.apply_oscillator_state(settings);
        self.advance_shared_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let generator_routes = self.generator_structural_route_frame;
        let aux_configs = self.aux_configs;
        let ratio_bands = structural_ratio_bands(groups, group_count, filters);
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            set_voice_swarm_clocks(voice, settings, self.swarm_time, self.secondary_swarm_time);
            voice.render_controlled_grouped::<DYNAMIC_UNISON>(
                settings,
                self.sample_rate,
                unison_control,
                &mut stems,
                oscillator_groups,
                group_count,
            );
            if ordered_filter_render {
                voice.render_ordered_oscillator_groups(
                    oscillator_bank,
                    settings,
                    self.sample_rate,
                    structural_control,
                    &mut stems,
                    groups,
                    group_count,
                    filters,
                    &ratio_bands,
                    &aux_configs,
                    &generator_routes,
                    None,
                );
            } else {
                voice.render_oscillator_bank_grouped(
                    oscillator_bank,
                    settings,
                    self.sample_rate,
                    structural_control,
                    &mut stems,
                    oscillator_groups,
                    group_count,
                );
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for stem in &mut stems {
            stem.0 *= MASTER_HEADROOM;
            stem.1 *= MASTER_HEADROOM;
        }
        stems
    }

    pub fn render_pair(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); 2] {
        if self.oscillator_bank.active()
            || !settings.legacy_primary_fast_path()
            || self.is_gliding()
                && self
                    .unison_settings
                    .iter()
                    .any(|settings| settings.motion_active())
            || self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .any(|voice| !voice.unison_transitions_steady())
        {
            return [
                self.render(settings, envelope),
                self.render(settings, envelope),
            ];
        }
        if self.active_count == 0 {
            return [(0.0, 0.0); 2];
        }
        self.configure_envelope(envelope);

        let clock0 = wrap_swarm_time(self.swarm_time + self.swarm_step);
        let clock1 = wrap_swarm_time(clock0 + self.swarm_step);
        self.swarm_time = clock1;
        let mut output = [(0.0_f32, 0.0_f32); 2];
        let mut rendered_second = false;
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let (samples, voice_rendered_second) =
                voice.render_pair(settings, self.sample_rate, [clock0 as f32, clock1 as f32]);
            rendered_second |= voice_rendered_second;
            for frame in 0..2 {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        if !rendered_second {
            self.swarm_time = clock0;
        }
        output
    }

    pub(crate) fn exact_saw_banks_eligible(&self, settings: VoiceSettings) -> bool {
        !self.oscillator_bank.active()
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(|voice| voice.exact_saw_banks_eligible(settings))
    }

    pub(crate) fn block_shape_banks_eligible(&self, settings: VoiceSettings) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(|voice| voice.block_shape_banks_eligible(settings))
    }

    pub(crate) fn morph_block_eligible(&self, _settings: VoiceSettings) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(VaVoice::unison_transitions_steady)
    }

    pub fn block_internal_samples(
        &self,
        _settings: VoiceSettings,
        oversampling_factor: u8,
    ) -> Option<usize> {
        let eligible =
            self.active_count != 0 && self.unison_layouts_steady() && !self.resynth_transitioning();
        eligible.then(|| {
            if oversampling_factor == 3 {
                FACTOR3_BLOCK_INTERNAL_SAMPLES
            } else {
                BLOCK_INTERNAL_SAMPLES
            }
        })
    }

    pub(crate) fn terminal_filter_block_eligible(
        &self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        group: &GeneratorRtGroup,
    ) -> bool {
        let Some(_) = group.terminal_filters() else {
            return false;
        };
        let oscillator_bank = self.oscillator_bank.render();
        self.active_count != 0
            && !self.has_active_resynth()
            && !self.resynth_transitioning()
            && self.oscillator_bank.active()
            && !self.oscillator_bank.transitioning()
            && settings
                .oscillators
                .iter()
                .all(|oscillator| !oscillator.enabled)
            && oscillator_bank.mask & !group.oscillator_mask() == 0
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(|voice| {
                    voice.terminal_filter_block_eligible(settings, oscillator_bank, envelope)
                })
    }

    pub(crate) fn render_terminal_filter_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        group: &GeneratorRtGroup,
        filters: &[FilterCoefficients; MAX_FILTERS],
    ) -> [(f32, f32); SAMPLES] {
        self.configure_envelope(envelope);
        debug_assert!(self.terminal_filter_block_eligible(settings, envelope, group));
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_terminal_filter_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                group,
                filters,
            );
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub(crate) fn render_terminal_filter_voice_modulated_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        group: &GeneratorRtGroup,
        base_filters: &[FilterConfig; MAX_FILTERS],
        shared_filters: &[FilterCoefficients; MAX_FILTERS],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(SAMPLES <= BLOCK_INTERNAL_SAMPLES);
        debug_assert!(self.voice_filter_modulation_only());
        debug_assert!(self.terminal_filter_block_eligible(settings, envelope, group));
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        let program = &self.voice_lfo_program;
        let routes = &self.voice_structural_route_frame;
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let rendered = voice.render_terminal_filter_voice_job::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                group,
                base_filters,
                shared_filters,
                Some(program),
                Some(routes),
            );
            for (output, rendered) in output.iter_mut().zip(rendered) {
                output.0 += rendered.0;
                output.1 += rendered.1;
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub fn render_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); SAMPLES] {
        if !self.oscillator_bank.active() && self.block_shape_banks_eligible(settings) {
            return self.render_saw_block(settings, envelope);
        }
        if self.oscillator_bank.transitioning() {
            return std::array::from_fn(|_| self.render(settings, envelope));
        }
        // A multi-chunk job commits to its chunk count before rendering, so the
        // last held voice can retire part-way through it. The remaining chunks
        // then render no voices at all, which is the correct silence rather
        // than a contract violation.
        debug_assert!(self.active_count != 0 || self.voices.iter().all(|voice| !voice.active()));
        self.configure_envelope(envelope);

        let mut clocks = [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT];
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                let (time, step) = if oscillator == 0 {
                    (&mut self.swarm_time, self.swarm_step)
                } else {
                    (
                        &mut self.secondary_swarm_time[oscillator - 1],
                        self.secondary_swarm_step[oscillator - 1],
                    )
                };
                for clock in &mut clocks[oscillator] {
                    *time = wrap_swarm_time(*time + step);
                    *clock = *time as f32;
                }
            }
        }
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        let oscillator_bank_active = self.oscillator_bank.active();
        let legacy_disabled = settings
            .oscillators
            .iter()
            .all(|oscillator| !oscillator.enabled);
        let settled_bank_config = legacy_disabled && oscillator_bank_active;
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = if oscillator_bank_active {
                voice.render_generic_block_with_static_oscillator_bank(
                    settings,
                    self.sample_rate,
                    clocks,
                    None,
                    self.oscillator_bank.render(),
                    legacy_disabled,
                    settled_bank_config,
                )
            } else {
                voice.render_generic_block(settings, self.sample_rate, clocks)
            };
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    fn grouped_block_eligible_with_dynamic_envelopes(
        &self,
        settings: VoiceSettings,
        dynamic_envelopes: bool,
    ) -> bool {
        self.active_count != 0
            && self.unison_layouts_steady()
            && !self.resynth_transitioning()
            && self.oscillator_bank.active()
            && !self.oscillator_bank.transitioning()
            && settings
                .oscillators
                .iter()
                .all(|oscillator| !oscillator.enabled)
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(|voice| {
                    if dynamic_envelopes {
                        voice.oscillator_bank_block_voice_eligible(self.oscillator_bank.render())
                    } else {
                        voice.settled_grouped_bank_voice_eligible(self.oscillator_bank.render())
                    }
                })
    }

    pub(crate) fn grouped_block_eligible(&self, settings: VoiceSettings) -> bool {
        self.grouped_block_eligible_with_dynamic_envelopes(settings, false)
    }

    pub(crate) fn dynamic_grouped_block_eligible(&self, settings: VoiceSettings) -> bool {
        self.grouped_block_eligible_with_dynamic_envelopes(settings, true)
    }

    pub(crate) fn render_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        oscillator_groups: &[u8; MAX_OSCILLATORS],
        group_count: usize,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.active_count == 0 || self.dynamic_grouped_block_eligible(settings));
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let rendered = voice.render_settled_oscillator_bank_grouped_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                oscillator_groups,
                group_count,
            );
            for group in 0..group_count {
                for frame in 0..SAMPLES {
                    output[group][frame].0 += rendered[group][frame].0;
                    output[group][frame].1 += rendered[group][frame].1;
                }
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for group in 0..group_count {
            for sample in &mut output[group] {
                sample.0 *= MASTER_HEADROOM;
                sample.1 *= MASTER_HEADROOM;
            }
        }
        output
    }

    pub(crate) fn render_ordered_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        filters: &[FilterCoefficients; MAX_FILTERS],
        filter_block: Option<&[[FilterCoefficients; MAX_FILTERS]]>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.active_count == 0 || self.dynamic_grouped_block_eligible(settings));
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let rendered = voice.render_ordered_oscillator_groups_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                groups,
                group_count,
                filters,
                filter_block,
                None,
            );
            for group in 0..group_count {
                for frame in 0..SAMPLES {
                    output[group][frame].0 += rendered[group][frame].0;
                    output[group][frame].1 += rendered[group][frame].1;
                }
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for group in 0..group_count {
            for sample in &mut output[group] {
                sample.0 *= MASTER_HEADROOM;
                sample.1 *= MASTER_HEADROOM;
            }
        }
        output
    }

    pub(crate) fn render_phase_mod_grouped_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        groups: &[GeneratorRtGroup],
        group_count: usize,
        controls: Option<&[StructuralOscillatorFrameControl]>,
        voice_modulation: bool,
        filters: &[FilterCoefficients; MAX_FILTERS],
        filter_block: Option<&[[FilterCoefficients; MAX_FILTERS]]>,
        generator_route_amounts: Option<(u8, &[f32])>,
    ) -> [[(f32, f32); SAMPLES]; MAX_OUTPUT_PAIRS] {
        debug_assert!(self.grouped_block_eligible(settings));
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let group_count = group_count.clamp(1, MAX_OUTPUT_PAIRS);
        let voice_modulation = voice_modulation.then_some((
            self.voice_lfo_program.as_ref(),
            &self.voice_structural_route_frame,
        ));
        let mut generator_routes = self.generator_structural_route_frame;
        let aux = self.aux_configs;
        generator_routes.retain_supported_filter_routes(filters);
        generator_routes.enable_fast_muted_sources(
            settings.fast_audio_rate_modulation && controls.is_none() && voice_modulation.is_none(),
            self.audible_oscillator_mask,
        );
        let generator_block = generator_routes.len != 0
            || generator_routes.filter_routes_active()
            || generator_routes.aux_routes_active();
        let mut output = [[(0.0_f32, 0.0_f32); SAMPLES]; MAX_OUTPUT_PAIRS];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let rendered = voice.render_phase_mod_grouped_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                groups,
                group_count,
                controls,
                voice_modulation,
                filters,
                filter_block,
                &aux,
                (generator_block
                    || controls.is_some()
                    || groups.iter().take(group_count).any(|group| {
                        group.modules().iter().any(|module| {
                            !matches!(module, crate::generators::GeneratorRtModule::Oscillator(_))
                        })
                    }))
                .then_some(&generator_routes),
                generator_route_amounts,
            );
            for group in 0..group_count {
                for frame in 0..SAMPLES {
                    output[group][frame].0 += rendered[group][frame].0;
                    output[group][frame].1 += rendered[group][frame].1;
                }
            }
        }
        for group in 0..group_count {
            for sample in &mut output[group] {
                sample.0 *= MASTER_HEADROOM;
                sample.1 *= MASTER_HEADROOM;
            }
        }
        output
    }

    pub(crate) fn structural_modulation_block_eligible(&self, settings: VoiceSettings) -> bool {
        self.active_count != 0
            && !self.has_active_resynth()
            && !self.resynth_transitioning()
            && self.oscillator_bank.active()
            && !self.oscillator_bank.transitioning()
            && settings
                .oscillators
                .iter()
                .all(|oscillator| !oscillator.enabled)
    }

    pub(crate) fn render_structural_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        controls: &[StructuralOscillatorFrameControl],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.structural_modulation_block_eligible(settings));
        debug_assert_eq!(controls.len(), SAMPLES);
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_structural_modulation_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                controls,
            );
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub(crate) fn render_voice_structural_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        controls: &[StructuralOscillatorFrameControl],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert!(self.voice_structural_modulation_block_eligible(settings));
        debug_assert_eq!(controls.len(), SAMPLES);
        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let oscillator_bank = self.oscillator_bank.render();
        let program = &self.voice_lfo_program;
        let routes = &self.voice_structural_route_frame;
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_voice_structural_modulation_block::<SAMPLES>(
                settings,
                self.sample_rate,
                oscillator_bank,
                controls,
                program,
                routes,
            );
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub fn render_saw_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
    ) -> [(f32, f32); SAMPLES] {
        if self.oscillator_bank.active() {
            return self.render_block(settings, envelope);
        }
        // A multi-chunk job commits to its chunk count before rendering, so the
        // last held voice can retire part-way through it. The remaining chunks
        // then render no voices at all, which is the correct silence rather
        // than a contract violation.
        debug_assert!(self.active_count != 0 || self.voices.iter().all(|voice| !voice.active()));
        self.configure_envelope(envelope);

        let mut clocks = [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT];
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                let (time, step) = if oscillator == 0 {
                    (&mut self.swarm_time, self.swarm_step)
                } else {
                    (
                        &mut self.secondary_swarm_time[oscillator - 1],
                        self.secondary_swarm_step[oscillator - 1],
                    )
                };
                for clock in &mut clocks[oscillator] {
                    *time = wrap_swarm_time(*time + step);
                    *clock = *time as f32;
                }
            }
        }
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_saw_block(settings, self.sample_rate, clocks);
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub fn render_morph_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        shapes: &[[f32; SAMPLES]; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        if self.oscillator_bank.active() {
            return std::array::from_fn(|frame| {
                let mut frame_settings = settings;
                for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                    frame_settings.oscillators[oscillator].shape = shapes[oscillator][frame];
                }
                self.render(frame_settings, envelope)
            });
        }
        debug_assert!(self.morph_block_eligible(settings));
        self.configure_envelope(envelope);
        let optimized = settings.oscillators.iter().all(|oscillator| {
            !oscillator.enabled || !oscillator.phase_warp_active() && !oscillator.custom_active()
        });
        let mut clocks = [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT];
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if settings.oscillator(oscillator).enabled {
                let (time, step) = if oscillator == 0 {
                    (&mut self.swarm_time, self.swarm_step)
                } else {
                    (
                        &mut self.secondary_swarm_time[oscillator - 1],
                        self.secondary_swarm_step[oscillator - 1],
                    )
                };
                for clock in &mut clocks[oscillator] {
                    *time = wrap_swarm_time(*time + step);
                    *clock = *time as f32;
                }
            }
        }
        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = if optimized {
                voice.render_morph_block(settings, self.sample_rate, clocks, shapes)
            } else {
                voice.render_generic_morph_block(settings, self.sample_rate, clocks, shapes)
            };
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub(crate) fn motion_block_eligible(&self, settings: VoiceSettings) -> bool {
        !self.oscillator_bank.active() && self.block_shape_banks_eligible(settings)
    }

    pub(crate) fn render_motion_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        modulation: &[crate::modulators::lfo::ModulationFrame],
        motion_mask: OscillatorMask,
        base_unison: &[UnisonSettings; LEGACY_OSCILLATOR_COUNT],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(modulation.len(), SAMPLES);
        debug_assert!(SAMPLES <= BLOCK_INTERNAL_SAMPLES);
        debug_assert!(self.active_count != 0);
        debug_assert!(self.motion_block_eligible(settings));

        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        let mut motion = [[UnisonMotionFrame::default(); SAMPLES]; LEGACY_OSCILLATOR_COUNT];
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if motion_mask & (1 << oscillator) == 0 {
                continue;
            }
            let base = base_unison[oscillator];
            for frame in 0..SAMPLES {
                let modulation = modulation[frame].unison[oscillator];
                let rate_scale = if modulation.jitter_rate_normalized == 0.0 {
                    1.0
                } else {
                    5_000.0_f32.powf(modulation.jitter_rate_normalized.clamp(-1.0, 1.0))
                };
                motion[oscillator][frame] = UnisonMotionFrame {
                    phase_random: (base.phase_random() + modulation.phase_random).clamp(0.0, 1.0),
                    swarm_amount: (base.swarm_amount() + modulation.jitter_amount).clamp(0.0, 1.0),
                    swarm_rate: (base.swarm_rate() * rate_scale).clamp(0.02, 100.0),
                };
            }
        }
        let mut swarm_clocks = [[0.0; SAMPLES]; LEGACY_OSCILLATOR_COUNT];
        let sample_rate = f64::from(self.sample_rate);
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if !settings.oscillator(oscillator).enabled {
                continue;
            }
            let (time, base_step) = if oscillator == 0 {
                (&mut self.swarm_time, self.swarm_step)
            } else {
                (
                    &mut self.secondary_swarm_time[oscillator - 1],
                    self.secondary_swarm_step[oscillator - 1],
                )
            };
            let dynamic = motion_mask & (1 << oscillator) != 0;
            for frame in 0..SAMPLES {
                let step = if dynamic {
                    f64::from(motion[oscillator][frame].swarm_rate) / sample_rate
                } else {
                    base_step
                };
                *time = wrap_swarm_time(*time + step);
                swarm_clocks[oscillator][frame] = *time as f32;
            }
        }

        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_motion_block(
                settings,
                self.sample_rate,
                swarm_clocks,
                &motion,
                motion_mask,
            );
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
            if motion_mask & (1 << oscillator) == 0 {
                continue;
            }
            let last = motion[oscillator][SAMPLES - 1];
            let settings = self.unison_settings[oscillator].with_motion(
                last.phase_random,
                last.swarm_amount,
                last.swarm_rate,
            );
            self.configure_unison_motion(oscillator, settings);
        }
        output
    }

    pub(crate) fn pitch_block_eligible(&self, settings: VoiceSettings) -> bool {
        !self.oscillator_bank.active()
            && self.exact_saw_banks_eligible(settings)
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(VaVoice::pitch_block_eligible)
    }

    pub(crate) fn spatial_block_eligible(&self) -> bool {
        self.voices
            .iter()
            .filter(|voice| voice.active())
            .all(VaVoice::spatial_block_eligible)
    }

    pub(crate) fn control_block_eligible(&self) -> bool {
        !self.oscillator_bank.transitioning()
            && self
                .voices
                .iter()
                .filter(|voice| voice.active())
                .all(VaVoice::control_block_eligible)
    }

    pub(crate) fn render_pitch_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        modulation: &[crate::modulators::lfo::ModulationFrame],
        unison_modulation_mask: OscillatorMask,
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(modulation.len(), SAMPLES);
        debug_assert!(SAMPLES <= BLOCK_INTERNAL_SAMPLES);
        debug_assert!(self.active_count != 0);
        debug_assert!(self.pitch_block_eligible(settings));

        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        for _ in 0..SAMPLES {
            if settings.oscillator(0).enabled {
                self.swarm_time = wrap_swarm_time(self.swarm_time + self.swarm_step);
            }
            for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
                if settings.oscillator(secondary + 1).enabled {
                    self.secondary_swarm_time[secondary] = wrap_swarm_time(
                        self.secondary_swarm_time[secondary] + self.secondary_swarm_step[secondary],
                    );
                }
            }
        }

        for (frame, modulation) in modulation.iter().enumerate() {
            let control = &mut self.pitch_block_controls[frame];
            control.oscillator_pitch_ratios = std::array::from_fn(|oscillator| {
                let base = settings.oscillator(oscillator).pitch_ratio;
                let semitones = modulation.oscillator[oscillator]
                    .pitch_semitones
                    .clamp(-96.0, 96.0);
                (base * (semitones / 12.0).exp2()).clamp(1.0 / 256.0, 256.0)
            });
            control.unison_active_mask = 0;
            control.unison_spatial_active_mask = 0;
        }
        if unison_modulation_mask != 0 {
            let mut frame_control = self
                .frame_control_cache
                .take()
                .expect("unison frame control cache must be initialized");
            for (frame, modulation) in modulation.iter().enumerate() {
                self.unison_frame_control(&modulation.unison, &mut frame_control);
                let control = &mut self.pitch_block_controls[frame];
                control.unison_active_mask = frame_control.active_mask;
                control.unison_spatial_active_mask = frame_control.spatial_shared_mask;
                for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                    let bit = 1 << oscillator;
                    if frame_control.active_mask & bit != 0 {
                        control.unison_pitch_correction[oscillator]
                            .copy_from_slice(&frame_control.pitch_correction[oscillator]);
                    }
                    if frame_control.spatial_shared_mask & bit != 0 {
                        control.unison_spatial_left[oscillator]
                            .copy_from_slice(&frame_control.spatial_left[oscillator]);
                        control.unison_spatial_right[oscillator]
                            .copy_from_slice(&frame_control.spatial_right[oscillator]);
                        control.unison_spatial_gain[oscillator] =
                            frame_control.spatial_gain[oscillator];
                    }
                }
            }
            self.frame_control_cache = Some(frame_control);
        }
        self.frame_control_valid = false;

        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let samples = voice.render_pitch_block::<SAMPLES>(
                settings,
                self.sample_rate,
                &self.pitch_block_controls[..SAMPLES],
            );
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0;
                output[frame].1 += samples[frame].1;
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        output
    }

    pub(crate) fn render_modulation_block<const SAMPLES: usize>(
        &mut self,
        settings: VoiceSettings,
        envelope: EnvelopeSettings,
        modulation: &[crate::modulators::lfo::ModulationFrame],
    ) -> [(f32, f32); SAMPLES] {
        debug_assert_eq!(modulation.len(), SAMPLES);
        debug_assert!(SAMPLES <= BLOCK_INTERNAL_SAMPLES);
        debug_assert!(self.active_count != 0);
        debug_assert!(self.control_block_eligible());

        self.configure_envelope(envelope);
        let settings = self.apply_oscillator_state(settings);
        for _ in 0..SAMPLES {
            if settings.oscillator(0).enabled {
                self.swarm_time = wrap_swarm_time(self.swarm_time + self.swarm_step);
            }
            for secondary in 0..LEGACY_OSCILLATOR_COUNT - 1 {
                if settings.oscillator(secondary + 1).enabled {
                    self.secondary_swarm_time[secondary] = wrap_swarm_time(
                        self.secondary_swarm_time[secondary] + self.secondary_swarm_step[secondary],
                    );
                }
            }
        }

        let frame_settings = std::array::from_fn(|frame| {
            let modulation = modulation[frame];
            let mut settings = settings;
            for oscillator in 0..LEGACY_OSCILLATOR_COUNT {
                let modulation = modulation.oscillator[oscillator];
                settings.modulate_oscillator(
                    oscillator,
                    modulation.pitch_semitones,
                    modulation.shape,
                    modulation.pulse_width,
                    modulation.warp,
                    modulation.custom_shape,
                    modulation.level,
                    modulation.pan,
                );
            }
            settings.velocity_amount =
                (settings.velocity_amount + modulation.global.velocity).clamp(0.0, 1.0);
            settings.pressure_amount =
                (settings.pressure_amount + modulation.global.pressure).clamp(0.0, 1.0);
            settings.timbre_amount =
                (settings.timbre_amount + modulation.global.timbre).clamp(0.0, 1.0);
            settings
        });
        let frame_envelopes = std::array::from_fn(|frame| {
            let modulation = modulation[frame].global;
            EnvelopeSettings {
                attack: (envelope.attack + modulation.attack).clamp(0.0, 8.0),
                decay: (envelope.decay + modulation.decay).clamp(0.0, 8.0),
                sustain: (envelope.sustain + modulation.sustain).clamp(0.0, 1.0),
                release: (envelope.release + modulation.release).clamp(0.0, 12.0),
                attack_curve: (envelope.attack_curve + modulation.attack_curve).clamp(-1.0, 1.0),
                decay_curve: (envelope.decay_curve + modulation.decay_curve).clamp(-1.0, 1.0),
                release_curve: (envelope.release_curve + modulation.release_curve).clamp(-1.0, 1.0),
                attack_curve_time: (envelope.attack_curve_time + modulation.attack_curve_time)
                    .clamp(0.05, 0.95),
                decay_curve_time: (envelope.decay_curve_time + modulation.decay_curve_time)
                    .clamp(0.05, 0.95),
                release_curve_time: (envelope.release_curve_time + modulation.release_curve_time)
                    .clamp(0.05, 0.95),
            }
        });
        self.frame_control_valid = false;

        let mut output = [(0.0_f32, 0.0_f32); SAMPLES];
        let voice_program = &self.voice_lfo_program;
        let voice_routes = &self.voice_route_frame;
        for voice in active_voices_mut(&mut self.voices, self.active_count) {
            let mut voice_output_gains = [1.0_f32; SAMPLES];
            let (voice_settings, voice_envelopes) = if voice_program.active() {
                let mut voice_settings = frame_settings;
                let mut voice_envelopes = frame_envelopes;
                for frame in 0..SAMPLES {
                    let values = voice.modulation.next(voice_program);
                    let modulation = voice_routes.evaluate_values(values);
                    apply_voice_settings(&mut voice_settings[frame], modulation);
                    voice_envelopes[frame] =
                        modulated_voice_envelope(voice_envelopes[frame], modulation.global);
                    voice_output_gains[frame] = voice_output_gain(modulation.global.output_db);
                }
                (voice_settings, voice_envelopes)
            } else {
                (frame_settings, frame_envelopes)
            };
            let samples = if self.oscillator_bank.active() {
                voice.render_oscillator_bank_modulation_block::<SAMPLES>(
                    &voice_settings,
                    &voice_envelopes,
                    self.sample_rate,
                    self.oscillator_bank.render(),
                )
            } else {
                voice.render_modulation_block::<SAMPLES>(
                    &voice_settings,
                    &voice_envelopes,
                    self.sample_rate,
                )
            };
            for frame in 0..SAMPLES {
                output[frame].0 += samples[frame].0 * voice_output_gains[frame];
                output[frame].1 += samples[frame].1 * voice_output_gains[frame];
            }
            retire_finished_voice(voice, &mut self.active_count, &mut self.resynth_playback);
        }
        for sample in &mut output {
            sample.0 *= MASTER_HEADROOM;
            sample.1 *= MASTER_HEADROOM;
        }
        self.envelope = frame_envelopes[SAMPLES - 1];
        output
    }

    pub const fn is_active(&self) -> bool {
        self.active_count != 0
    }

    pub const fn is_gliding(&self) -> bool {
        self.voice_mode == 1 && self.voices[0].is_gliding()
    }

    pub const fn latest_stereo_seed(&self, oscillator: usize) -> f32 {
        self.latest_stereo_seed[oscillator]
    }

    pub const fn swarm_time(&self) -> f32 {
        self.swarm_time as f32
    }

    pub const fn secondary_swarm_time(&self, oscillator: usize) -> f32 {
        self.secondary_swarm_time[oscillator - 1] as f32
    }

    fn set_latest_stereo_seeds(&mut self, seed: u64) {
        self.latest_stereo_seed =
            std::array::from_fn(|oscillator| oscillator_stereo_seed(seed, oscillator));
    }

    fn effective_pitch_bend(&self, channel: u8) -> f32 {
        self.parameter_bend
            + self.pitch_bend[0]
            + if channel == 0 {
                0.0
            } else {
                self.pitch_bend[channel as usize]
            }
    }

    fn clear_mono_channel(&mut self, channel: u8) {
        let len = usize::from(self.mono_stack_len);
        let mut output = 0;
        for index in 0..len {
            let held = self.mono_stack[index];
            if channel != 0 && held.channel != channel {
                self.mono_stack[output] = held;
                output += 1;
            }
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the fixed mono note stack contains at most 32 entries"
        )]
        {
            self.mono_stack_len = output as u8;
        }
    }

    fn effective_timbre(&self, channel: u8) -> f32 {
        let member = if channel == 0 {
            0.5
        } else {
            self.timbre[channel as usize]
        };
        (self.timbre[0] + member - 0.5).clamp(0.0, 1.0)
    }

    fn refresh_voice_count(&mut self) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the fixed pool has exactly 32 note voices"
        )]
        {
            self.active_count = self.voices.iter().filter(|voice| voice.active()).count() as u8;
        }
        self.settle_resynth_playback_if_idle();
    }
}

#[cfg(test)]
mod structural_control_tests {
    use super::*;

    #[test]
    fn generator_route_schedule_accepts_reverse_self_and_cycles() {
        let target = |slot, control| crate::ResolvedModularTarget::Oscillator { slot, control };

        let mut reverse = GeneratorStructuralRouteFrame::default();
        reverse.push(1, 1.0, 0, target(0, crate::OscillatorControl::Level));
        reverse.finish();
        let source = reverse
            .order()
            .iter()
            .position(|&slot| slot == 1)
            .expect("source in schedule");
        let carrier = reverse
            .order()
            .iter()
            .position(|&slot| slot == 0)
            .expect("carrier in schedule");
        assert!(source < carrier);

        let mut feedback = GeneratorStructuralRouteFrame::default();
        feedback.push(
            0,
            1.0,
            0,
            target(0, crate::OscillatorControl::RingModAmount),
        );
        feedback.push(
            0,
            1.0,
            1,
            target(1, crate::OscillatorControl::PhasePosition),
        );
        feedback.push(
            1,
            1.0,
            2,
            target(0, crate::OscillatorControl::PhasePosition),
        );
        feedback.finish();
        assert_eq!(feedback.feedback_source_mask(), 0b11);
    }

    #[test]
    fn generator_filter_route_applies_audio_rate_depth() {
        let mut routes = GeneratorStructuralRouteFrame::default();
        routes.push(
            0,
            0.25,
            16,
            crate::ResolvedModularTarget::Filter {
                slot: 0,
                control: crate::FilterControl::Cutoff,
            },
        );
        routes.push_depth(1, 0.5, 16);
        routes.finish();

        let mut sources = [0.0; MAX_OSCILLATORS];
        sources[0] = 0.4;
        sources[1] = 0.2;
        let delta = routes.filter_delta(0, &sources, None);
        assert!((delta.cutoff_octaves - 0.56).abs() < f32::EPSILON);
        assert!(routes.gain_block_eligible());
        assert!(routes.single_filter_route().is_none());
    }

    fn resynth_test_tone(frequency: f32) -> Vec<u8> {
        crate::wav_test::wav_i16(
            1,
            48_000,
            (0..2_048).map(|index| {
                let sample = (std::f32::consts::TAU * frequency * index as f32 / 48_000.0).sin();
                (sample * 20_000.0) as i16
            }),
        )
    }

    fn install_resynth_test_artifact(
        assets: &crate::resynth_state::ResynthAssetPackState,
        slot: usize,
        frequency: f32,
    ) -> u64 {
        let controls = crate::oscillators::ResynthControls::default();
        let model =
            crate::oscillators::analyze_wav("batch.wav", resynth_test_tone(frequency), controls)
                .expect("analyze");
        assets
            .slot(slot)
            .expect("slot")
            .replace(
                model,
                crate::oscillators::ResynthAlgorithm::Sample,
                controls,
            )
            .expect("publish")
    }

    #[test]
    fn resynth_batch_preflight_rejects_without_partial_plan_mutation() {
        let assets = crate::resynth_state::ResynthAssetPackState::new();
        let first_0 = install_resynth_test_artifact(&assets, 0, 110.0);
        let first_1 = install_resynth_test_artifact(&assets, 1, 220.0);
        let mut first = ResynthRtUpdate {
            changed_mask: (1_u32 << 0) | (1_u32 << 1),
            views: [crate::resynth_state::ResynthArtifactView::NONE; MAX_OSCILLATORS],
        };
        first.views[0] = assets
            .slot(0)
            .expect("slot 0")
            .try_rt_view_after(0)
            .expect("view 0");
        first.views[1] = assets
            .slot(1)
            .expect("slot 1")
            .try_rt_view_after(0)
            .expect("view 1");
        let mut synth = PolySynth::default();
        synth.active_count = 1;
        assert!(synth.try_retarget_resynth_batch(&first));

        let second_0 = install_resynth_test_artifact(&assets, 0, 330.0);
        let mut begin_transition = ResynthRtUpdate {
            changed_mask: 1,
            views: [crate::resynth_state::ResynthArtifactView::NONE; MAX_OSCILLATORS],
        };
        begin_transition.views[0] = assets
            .slot(0)
            .expect("slot 0")
            .try_rt_view_after(first_0)
            .expect("second view 0");
        assert!(synth.try_retarget_resynth_batch(&begin_transition));
        assert_eq!(synth.resynth_playback[0].to.generation(), second_0);
        assert_ne!(synth.resynth_playback[0].remaining, 0);

        let third_0 = install_resynth_test_artifact(&assets, 0, 440.0);
        let second_1 = install_resynth_test_artifact(&assets, 1, 550.0);
        let mut rejected = ResynthRtUpdate {
            changed_mask: (1_u32 << 0) | (1_u32 << 1),
            views: [crate::resynth_state::ResynthArtifactView::NONE; MAX_OSCILLATORS],
        };
        rejected.views[0] = assets
            .slot(0)
            .expect("slot 0")
            .try_rt_view_after(second_0)
            .expect("third view 0");
        rejected.views[1] = assets
            .slot(1)
            .expect("slot 1")
            .try_rt_view_after(first_1)
            .expect("second view 1");
        assert_eq!(rejected.views[0].generation(), third_0);
        assert_eq!(rejected.views[1].generation(), second_1);
        assert!(!synth.try_retarget_resynth_batch(&rejected));
        assert_eq!(synth.resynth_playback[0].to.generation(), second_0);
        assert_eq!(synth.resynth_playback[1].to.generation(), first_1);
    }

    #[test]
    fn natural_idle_settlement_preserves_held_source_audition() {
        let mut synth = PolySynth::default();
        synth.resynth_playback[0].snap_source_audition(true);
        synth.active_count = 0;
        synth.settle_resynth_playback_if_idle();
        assert_eq!(synth.resynth_playback[0].source_mix, 1.0);
    }

    fn oscillator_config() -> OscillatorDspConfig {
        OscillatorDspConfig {
            enabled: true,
            engine: crate::generators::OscillatorEngineKind::Va,
            resynth_playback: ResynthPlaybackPtr::NONE,
            shape: 2.0,
            pulse_width: 0.5,
            custom_curve: crate::wave_curve::WaveCurveRt::zero(),
            custom_mix: 0.0,
            positioned_wave: false,
            phase_warp_mode: 0,
            phase_warp_amount: 0.0,
            phase_mod_source: 0,
            phase_mod_amount: 0.0,
            modulation_mode: crate::generators::GeneratorModMode::Phase,
            tuning_mode: crate::generators::OscillatorTuningMode::Semicent,
            frequency_offset_hz: 0.0,
            frequency_ratio: 1.0,
            transpose: 0.0,
            cents: 0.0,
            level: 0.5,
            pan: 0.0,
            unison_voices: 4,
            unison_range: 1.0,
            unison_amount: 1.0,
            unison_curve: 0.0,
            unison_jitter: 0.0,
            unison_jitter_mode: 0,
            unison_rate: 0.4,
            unison_weight: 0.0,
            unison_width: 1.0,
            phase_position: 0.0,
            phase_random: 0.0,
            unison_alignment: 0.0,
            unison_alignment_mode: 0,
            unison_pan_curve: 0.0,
            unison_pan_center_x: 0.5,
            unison_pan_segments: (
                crate::pan_curve::PanShapeSegmentsRt::default(),
                crate::pan_curve::PanShapeSegmentsRt::default(),
            ),
            // Deliberately differ from the monophonic frame settings below.
            unison_stereo_x: 0.1,
            unison_stereo_alternate: 0.1,
        }
    }

    #[test]
    fn monophonic_and_polyphonic_pan_xy_deltas_are_combined() {
        let mut bank = ActiveOscillatorSet::default();
        bank.configured[0] = Some(oscillator_config());
        let mut base = StructuralOscillatorFrameControl::NEUTRAL;
        base.mask = 1;
        base.slots[0].stereo_x = 0.2;
        base.slots[0].stereo_y = 0.3;
        let mut voice = crate::StructuralModulationFrame::default();
        voice.oscillator_mask = 1;
        voice.oscillators[0].stereo_x = 0.4;
        voice.oscillators[0].stereo_y = 0.5;

        let merged = merge_voice_structural_control(&base, &voice, &bank);
        assert!((merged.slots[0].stereo_x - 0.6).abs() < f32::EPSILON);
        assert!((merged.slots[0].stereo_y - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn reset_preserves_monotonic_resynth_monitor_identities() {
        let mut synth = PolySynth::default();
        let (publish_before, audio_before) = synth.begin_resynth_telemetry_block(64);
        synth.reset();
        let (publish_after, audio_after) = synth.begin_resynth_telemetry_block(64);
        assert_eq!(publish_after, publish_before.wrapping_add(1));
        assert_eq!(audio_after, audio_before.wrapping_add(64));
    }

    #[test]
    fn positioned_waves_ignore_persisted_voice_shape_routes() {
        let mut config = oscillator_config();
        config.positioned_wave = true;
        let mut bank = ActiveOscillatorSet::default();
        bank.configured[0] = Some(config);
        let mut base = StructuralOscillatorFrameControl::NEUTRAL;
        base.mask = 1;
        base.slots[0].shape = 1.25;
        let mut voice = crate::StructuralModulationFrame::default();
        voice.oscillator_mask = 1;
        voice.oscillators[0].shape = 1.0;

        let merged = merge_voice_structural_control(&base, &voice, &bank);
        assert_eq!(merged.slots[0].shape, 1.25);

        let configs = std::array::from_fn(|slot| {
            let mut config = oscillator_config();
            config.enabled = slot == 0;
            config.positioned_wave = slot == 0;
            config
        });
        bank.configure(configs, 48_000.0);
        bank.render.entry_mut(0).current.positioned_wave = true;
        let block_merged = merge_voice_structural_block_control(&base, &voice, &bank.render);
        assert_eq!(block_merged.slots[0].shape, 1.25);
    }

    fn bank_oscillator(enabled: bool, transpose: f32) -> OscillatorDspConfig {
        let mut config = oscillator_config();
        config.enabled = enabled;
        config.transpose = transpose;
        config.unison_voices = 1;
        config
    }

    fn two_group_synth() -> (
        PolySynth,
        VoiceSettings,
        EnvelopeSettings,
        [u8; MAX_OSCILLATORS],
    ) {
        let mut synth = PolySynth::default();
        synth.set_sample_rate(48_000.0);
        synth.configure_oscillator_enabled([false, false, false]);
        let mut configs = std::array::from_fn(|_| bank_oscillator(false, 0.0));
        configs[0] = bank_oscillator(true, 0.0);
        configs[1] = bank_oscillator(true, 7.0);
        synth.configure_oscillators(configs);
        synth.configure_output_groups(
            [EnvelopeSettings::default(); MAX_OUTPUT_PAIRS],
            [0; MAX_OUTPUT_PAIRS],
            2,
            0b11,
            true,
        );
        synth.note_on(60, 1.0, 0, None);
        let mut settings = VoiceSettings::new(2.0, 261.63, 0.5, 0.0, 0.0, 0.0);
        settings.oscillators[0].enabled = false;
        let mut oscillator_groups = [0_u8; MAX_OSCILLATORS];
        oscillator_groups[1] = 1;
        (
            synth,
            settings,
            EnvelopeSettings::default(),
            oscillator_groups,
        )
    }

    #[test]
    fn grouped_block_keeps_oscillators_on_their_group_stems() {
        let (mut synth, settings, envelope, oscillator_groups) = two_group_synth();
        // A zero-attack group envelope steps straight from silence to full
        // level, so the gain declicker spreads a band-limited residual over the
        // first samples of the note. The amplitude genuinely moves there, so
        // only the dynamic block path is legal until the residual drains.
        assert!(synth.dynamic_grouped_block_eligible(settings));
        assert!(!synth.grouped_block_eligible(settings));
        let neutral_groups = [GeneratorRtGroup::EMPTY; MAX_OUTPUT_PAIRS];
        let neutral_filters = [FilterCoefficients::default(); MAX_FILTERS];
        for _ in 0..crate::voices::declick::transition_samples() {
            synth.render_grouped_neutral(
                settings,
                envelope,
                &oscillator_groups,
                2,
                &neutral_groups,
                &neutral_filters,
                false,
            );
        }
        assert!(synth.grouped_block_eligible(settings));
        let stems = synth.render_grouped_block::<BLOCK_INTERNAL_SAMPLES>(
            settings,
            envelope,
            &oscillator_groups,
            2,
        );
        let energy = |group: usize| {
            stems[group]
                .iter()
                .map(|(left, right)| left * left + right * right)
                .sum::<f32>()
        };
        let group_0 = energy(0);
        let group_1 = energy(1);
        let leaked = (2..MAX_OUTPUT_PAIRS).map(energy).sum::<f32>();
        assert!(group_0 > 1.0e-6, "group 0 silent: {group_0}");
        assert!(group_1 > 1.0e-6, "group 1 silent: {group_1}");
        assert!(
            leaked < 1.0e-12,
            "energy leaked into unused stems: {leaked}"
        );
    }

    #[test]
    fn grouped_block_matches_per_sample_grouped_render() {
        let (mut block_synth, settings, envelope, oscillator_groups) = two_group_synth();
        let (mut sample_synth, _, _, _) = two_group_synth();
        let groups = [GeneratorRtGroup::EMPTY; MAX_OUTPUT_PAIRS];
        let filters = [FilterCoefficients::default(); MAX_FILTERS];
        let block = block_synth.render_grouped_block::<BLOCK_INTERNAL_SAMPLES>(
            settings,
            envelope,
            &oscillator_groups,
            2,
        );
        let mut sample = [[(0.0_f32, 0.0_f32); BLOCK_INTERNAL_SAMPLES]; MAX_OUTPUT_PAIRS];
        for frame in 0..BLOCK_INTERNAL_SAMPLES {
            let rendered = sample_synth.render_grouped_neutral(
                settings,
                envelope,
                &oscillator_groups,
                2,
                &groups,
                &filters,
                false,
            );
            for group in 0..2 {
                sample[group][frame] = rendered[group];
            }
        }
        for group in 0..2 {
            let mut error = 0.0_f32;
            let mut energy = 0.0_f32;
            for frame in 0..BLOCK_INTERNAL_SAMPLES {
                let (block_left, block_right) = block[group][frame];
                let (sample_left, sample_right) = sample[group][frame];
                error += (block_left - sample_left).abs() + (block_right - sample_right).abs();
                energy += sample_left.abs() + sample_right.abs();
            }
            assert!(
                error <= energy * 1.0e-3 + 1.0e-5,
                "group {group} block drifted from per-sample render: error={error} energy={energy}"
            );
        }
    }
}
