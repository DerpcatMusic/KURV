use super::*;

pub(super) fn reorder_insertion(
    ui: &egui::Ui,
    visible_sources: &[usize],
    collapsed_modulators: u64,
    reserved: usize,
    pointer: egui::Pos2,
) -> usize {
    let gap = ui.spacing().item_spacing.y;
    let indicator_height = reorder_indicator_height(ui);
    let mut top = ui.cursor().top();
    for (insertion, index) in visible_sources.iter().enumerate() {
        if reserved == insertion {
            let indicator_bottom = top + indicator_height + gap;
            if pointer.y < indicator_bottom {
                return insertion;
            }
            top = indicator_bottom;
        }
        let height = if collapsed_modulators & (1_u64 << *index) != 0 {
            collapsed_module_height(ui)
        } else {
            expanded_module_height(ui)
        };
        if pointer.y < top + height * 0.5 {
            return insertion;
        }
        top += height + gap;
    }
    visible_sources.len()
}

pub(super) fn draw_reorder_insertion(ui: &mut egui::Ui, width: f32, source_slot: usize) {
    let color = source_color(source_slot);
    let height = reorder_indicator_height(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let inset = editor_theme::space::XS.min(rect.width() * 0.04);
    let line = [
        egui::pos2(rect.left() + inset, rect.center().y),
        egui::pos2(rect.right() - inset, rect.center().y),
    ];
    ui.painter().rect_filled(
        rect,
        editor_theme::shape::CONTROL_RADIUS,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 18),
    );
    ui.painter().line_segment(
        line,
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
    );
    for point in line {
        ui.painter()
            .circle_filled(point, editor_theme::shape::FOCUS_STROKE * 1.25, color);
    }
}

fn reorder_indicator_height(ui: &egui::Ui) -> f32 {
    (editor_theme::compact_gap(ui) * 2.0).max(editor_theme::title_height(ui) * 0.28)
}

pub(super) fn nearest_modulator_insertion(
    ui: &egui::Ui,
    visible_sources: &[usize],
    collapsed_modulators: u64,
    reserved: Option<usize>,
) -> Option<usize> {
    let (alt, pointer) = ui.input(|input| (input.modifiers.alt, input.pointer.latest_pos()));
    if !alt
        || ui.ctx().dragged_id().is_some()
        || egui::DragAndDrop::has_payload_of_type::<crate::generators::ModuleId>(ui.ctx())
        || egui::DragAndDrop::has_payload_of_type::<crate::generators::GroupId>(ui.ctx())
        || crate::editor_modulation::source_drag_active(ui)
    {
        return None;
    }
    let pointer = pointer?;
    let rack = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    if !rack.contains(pointer) {
        return None;
    }
    let row_height = editor_theme::title_height(ui);
    let gap = ui.spacing().item_spacing.y;
    let mut edge = ui.cursor().top();
    let mut nearest = None;
    let pointer_in_use = ui.ctx().egui_is_using_pointer();
    for (insertion, index) in visible_sources.iter().enumerate() {
        if reserved == Some(insertion)
            && (edge - row_height * 0.16..=edge + row_height).contains(&pointer.y)
        {
            return Some(insertion);
        }
        if !pointer_in_use {
            let distance = (pointer.y - edge).abs();
            if nearest.is_none_or(|(_, nearest_distance)| distance < nearest_distance) {
                nearest = Some((insertion, distance));
            }
        }
        if reserved == Some(insertion) {
            edge += row_height + gap;
        }
        edge += if collapsed_modulators & (1_u64 << *index) != 0 {
            collapsed_module_height(ui)
        } else {
            expanded_module_height(ui)
        } + gap;
    }
    nearest
        .filter(|(_, distance)| *distance <= row_height * 0.72)
        .map(|(insertion, _)| insertion)
}

pub(super) fn place_source_at_active_insertion(
    state: &PluginContext<KurvParams>,
    source_slot: usize,
    active: u64,
    presentation_insertion: usize,
) {
    let order = state.params().modulator_rack.presentation_order();
    let active_slots: Vec<_> = order
        .iter()
        .copied()
        .filter(|slot| active & (1_u64 << *slot) != 0)
        .collect();
    let full_insertion = if let Some(target) = active_slots.get(presentation_insertion) {
        order
            .iter()
            .position(|slot| slot == target)
            .unwrap_or_default()
    } else {
        active_slots
            .last()
            .and_then(|target| order.iter().position(|slot| slot == target))
            .map_or(0, |position| position + 1)
    };
    state
        .params()
        .modulator_rack
        .move_source_slot(source_slot, full_insertion);
}

pub(super) fn duplicate_source_at_active_insertion(
    state: &PluginContext<KurvParams>,
    source_slot: usize,
    active: &mut u64,
    presentation_insertion: usize,
) -> Option<usize> {
    let destination = (LEGACY_MODULATION_SOURCES..MAX_MODULATION_SOURCES)
        .find(|index| *active & (1_u64 << index) == 0)?;
    let config = source_config(state, source_slot);
    let curve = if source_slot < LEGACY_MODULATION_SOURCES {
        lfo_curve(state.params(), source_slot).snapshot()
    } else {
        state.params().modulator_rack.curve(source_slot)?.snapshot()
    };
    place_source_at_active_insertion(state, destination, *active, presentation_insertion);
    state
        .params()
        .modulator_rack
        .set_config(destination, config);
    state
        .params()
        .modulator_rack
        .curve(destination)?
        .replace(curve);
    *active |= 1_u64 << destination;
    Some(destination)
}
