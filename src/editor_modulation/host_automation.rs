//! Host-visible automation assignment and frame-local editor lookup.

use super::*;
use crate::modulators::routing::HOST_AUTOMATION_SLOT_COUNT;
use crate::params::HOST_AUTOMATION_PARAMS;

const CACHE_ID: &str = "kurv-host-automation-targets";

#[derive(Clone)]
struct FrameCache {
    frame: u64,
    assignments: [Option<(ModulationRouteTarget, usize)>; HOST_AUTOMATION_SLOT_COUNT],
    assignment_len: usize,
    last_target: Option<ModulationRouteTarget>,
    last_slot: Option<usize>,
}

impl Default for FrameCache {
    fn default() -> Self {
        Self {
            frame: u64::MAX,
            assignments: [None; HOST_AUTOMATION_SLOT_COUNT],
            assignment_len: 0,
            last_target: None,
            last_slot: None,
        }
    }
}

pub(crate) fn host_automation_destination(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
    base: f32,
) {
    if source_drag_active(ui) {
        return;
    }
    response.context_menu(|ui| host_automation_menu(ui, state, target, base));
    paint_badge(ui, state, target, response);
}

pub(crate) fn update_host_automation_gesture(
    state: &PluginContext<KurvParams>,
    param: P,
    response: &egui::Response,
    normalized: f32,
    changed: bool,
) {
    let gesture = response.drag_started() || response.dragged() || response.drag_stopped();
    if response.drag_started() {
        state.begin_edit(param);
    }
    if changed {
        if gesture {
            state.set_param(param, f64::from(normalized.clamp(0.0, 1.0)));
        } else {
            state.begin_edit(param);
            state.set_param(param, f64::from(normalized.clamp(0.0, 1.0)));
            state.end_edit(param);
        }
    }
    if response.drag_stopped() {
        state.end_edit(param);
    }
}

pub(crate) fn host_automation_menu(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    base: f32,
) {
    let targets = state.params().host_automation_targets.snapshot();
    let assigned = slot_in(&targets, target);
    ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
    ui.set_min_width(editor_theme::title_height(ui) * 7.0);
    ui.label(
        egui::RichText::new(target_label(UiDestination::Modular(target)))
            .font(editor_theme::font::caption())
            .color(editor_theme::semantic().text_muted),
    );
    if let Some(slot) = assigned {
        ui.label(
            egui::RichText::new(format!("HOST {:02}", slot + 1))
                .font(editor_theme::font::value())
                .color(editor_theme::semantic().primary),
        );
        if ui.button("Remove host assignment").clicked() {
            let normalized = state.get_param(HOST_AUTOMATION_PARAMS[slot]);
            commit_value(state, target, normalized);
            state.params().host_automation_targets.clear(slot);
            ui.close();
        }
    } else if let Some(slot) = targets.iter().position(Option::is_none) {
        if ui.button("Make host modulatable").clicked() {
            let param = HOST_AUTOMATION_PARAMS[slot];
            state.begin_edit(param);
            state.set_param(param, f64::from(base.clamp(0.0, 1.0)));
            state.end_edit(param);
            state.params().host_automation_targets.set(slot, target);
            ui.close();
        }
    } else {
        ui.add_enabled(false, egui::Button::new("Host automation bank full"));
    }
}

pub(crate) fn host_automation_binding(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> Option<(usize, P, f32)> {
    if source_drag_active(ui) {
        return None;
    }
    let slot = slot(ui, state, target)?;
    let param = HOST_AUTOMATION_PARAMS[slot];
    Some((slot, param, state.get_param(param).clamp(0.0, 1.0)))
}

fn paint_badge(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
    response: &egui::Response,
) {
    let Some(slot) = slot(ui, state, target) else {
        return;
    };
    let visible = response.rect.intersect(ui.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let accent = editor_theme::semantic().primary;
    let color = accent.gamma_multiply(if response.hovered() { 1.0 } else { 0.68 });
    let galley = ui.painter().layout_no_wrap(
        format!("H{:02}", slot + 1),
        editor_theme::font::caption(),
        color,
    );
    let padding = egui::vec2(editor_theme::space::XXS, editor_theme::shape::STROKE);
    let size = galley.size() + padding * 2.0;
    if visible.width() < size.x + editor_theme::space::XXS
        || visible.height() < size.y + editor_theme::space::XXS
    {
        return;
    }
    let rect = egui::Rect::from_min_size(
        visible.right_top()
            + egui::vec2(-size.x - editor_theme::space::XXS, editor_theme::space::XXS),
        size,
    );
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        editor_theme::semantic().well,
    );
    ui.painter().rect_stroke(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Stroke::new(editor_theme::shape::STROKE, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(rect.min + padding, galley, color);
}

fn slot(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    target: ModulationRouteTarget,
) -> Option<usize> {
    let frame = ui.ctx().cumulative_frame_nr();
    let id = egui::Id::new(CACHE_ID);
    let stale = ui.data_mut(|data| data.get_temp_mut_or_default::<FrameCache>(id).frame != frame);
    if stale {
        let targets = state.params().host_automation_targets.snapshot();
        ui.data_mut(|data| {
            let cache = data.get_temp_mut_or_default::<FrameCache>(id);
            cache.frame = frame;
            cache.assignment_len = 0;
            cache.last_target = None;
            cache.last_slot = None;
            for (slot, target) in targets.into_iter().enumerate() {
                let Some(target) = target else {
                    continue;
                };
                cache.assignments[cache.assignment_len] = Some((target, slot));
                cache.assignment_len += 1;
            }
        });
    }
    ui.data_mut(|data| {
        let cache = data.get_temp_mut_or_default::<FrameCache>(id);
        if cache.last_target != Some(target) {
            cache.last_target = Some(target);
            cache.last_slot = cache.assignments[..cache.assignment_len]
                .iter()
                .flatten()
                .find_map(|(candidate, slot)| (*candidate == target).then_some(*slot));
        }
        cache.last_slot
    })
}

fn slot_in(
    targets: &[Option<ModulationRouteTarget>; HOST_AUTOMATION_SLOT_COUNT],
    target: ModulationRouteTarget,
) -> Option<usize> {
    targets
        .iter()
        .position(|candidate| *candidate == Some(target))
}

fn commit_value(state: &PluginContext<KurvParams>, target: ModulationRouteTarget, normalized: f32) {
    match target {
        ModulationRouteTarget::Legacy { .. } => {}
        ModulationRouteTarget::Oscillator {
            module_id,
            slot,
            control,
        } => {
            let patch = state.generator_stack.snapshot();
            let valid = patch.groups().iter().any(|group| {
                group.modules().iter().any(|module| {
                    module.id().get() == module_id && module.oscillator_slot() == Some(slot)
                })
            });
            if valid {
                let mut config = state.generator_stack.oscillator_config(slot);
                control.apply_normalized(&mut config, normalized);
                state.generator_stack.set_oscillator_config(slot, config);
            }
        }
        ModulationRouteTarget::Group { group_id, control } => {
            let patch = state.generator_stack.snapshot();
            if let Some(group) = patch
                .groups()
                .iter()
                .find(|group| group.id().get() == group_id)
            {
                let mut output = group.output();
                control.apply_normalized(&mut output, normalized);
                state.generator_stack.set_group_output(group.id(), output);
            }
        }
        ModulationRouteTarget::Filter {
            module_id,
            slot,
            control,
        } => {
            let patch = state.generator_stack.snapshot();
            let valid = patch.groups().iter().any(|group| {
                group.modules().iter().any(|module| {
                    module.id().get() == module_id && module.filter_slot() == Some(slot)
                })
            });
            if valid {
                let mut config = state.generator_stack.filter_config(slot);
                control.apply_normalized(&mut config, normalized);
                state.generator_stack.set_filter_config(slot, config);
            }
        }
    }
}
