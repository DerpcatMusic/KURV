use truce_core::editor::PluginContext;

use crate::KurvParams;
use crate::editor_controls::fit_font_to_width;
use crate::editor_theme;
use crate::editor_widgets::icon_font_ready;
use crate::filters::{MAX_RATIO, MIN_RATIO, StereoTptSvf, ratio_brickwall_bypassed};
use crate::generators::{
    GroupId, GroupOutput, MAX_GENERATOR_MODULES, MAX_OSCILLATORS, Module, ModuleKind,
    OscillatorEngineKind,
};
use crate::modulators::routing::OscillatorControl;
use crate::oscillators::{PhaseWarpMode, VaOscillator};

use super::super::translucent;
use super::GroupOutputInteraction;

pub(crate) fn clear_group_name_edit_state(ui: &egui::Ui, group_id: GroupId) {
    let rename_id = egui::Id::new(("generator-group-rename", group_id.get()));
    let editor_id = egui::Id::new(("generator-group-editor", group_id.get()));
    ui.data_mut(|data| {
        data.remove::<bool>(rename_id);
        data.remove::<String>(rename_id.with("draft"));
        data.remove::<bool>(editor_id);
    });
}

pub(crate) fn group_editor_open(ui: &egui::Ui, group_id: GroupId) -> bool {
    ui.data(|data| {
        data.get_temp::<bool>(egui::Id::new(("generator-group-editor", group_id.get())))
            .unwrap_or(false)
    })
}

pub(crate) fn draw_group_editor_panel(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: GroupId,
    group_accent: egui::Color32,
) -> Option<egui::Color32> {
    if !rect.is_positive() {
        return None;
    }
    let palette = editor_theme::semantic();
    let rename_id = egui::Id::new(("generator-group-rename", group_id.get()));
    let group_label = state
        .params()
        .editor_state
        .lock()
        .ok()
        .and_then(|editor| editor.group_name(group_id.get()).map(str::to_owned))
        .unwrap_or_else(|| format!("Group {}", group_id.get()));
    let mut draft = ui
        .data(|data| data.get_temp::<String>(rename_id.with("draft")))
        .unwrap_or(group_label);
    let focus_name = ui
        .data_mut(|data| data.remove_temp::<bool>(rename_id.with("focus")))
        .unwrap_or(false);
    let mut selected = group_accent;
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XS);
    let popup_width = (editor_theme::title_height(ui) * 9.0).min(screen.width());
    let popup_x = rect.left().clamp(
        screen.left(),
        (screen.right() - popup_width).max(screen.left()),
    );
    let popup = egui::Area::new(rename_id.with("popup"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(popup_x, rect.bottom()))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(editor_theme::shape::STROKE, group_accent))
                .corner_radius(editor_theme::shape::CONTROL_RADIUS)
                .inner_margin(egui::Margin::same(editor_theme::space::XS as i8))
                .show(ui, |ui| {
                    ui.set_min_width(popup_width);
                    ui.spacing_mut().slider_width = popup_width - editor_theme::space::XS * 2.0;
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut draft)
                            .id_salt(rename_id.with("field"))
                            .desired_width(ui.available_width())
                            .font(editor_theme::font::title()),
                    );
                    if focus_name {
                        edit.request_focus();
                    }
                    let pulse = ((ui.input(|input| input.time) * std::f64::consts::TAU * 1.4).sin()
                        as f32
                        + 1.0)
                        * 0.5;
                    ui.painter().rect_stroke(
                        edit.rect,
                        editor_theme::shape::CONTROL_RADIUS,
                        egui::Stroke::new(
                            editor_theme::shape::FOCUS_STROKE,
                            group_accent.gamma_multiply(0.45 + pulse * 0.55),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    ui.add_space(editor_theme::compact_gap(ui));
                    egui::widgets::color_picker::color_picker_color32(
                        ui,
                        &mut selected,
                        egui::widgets::color_picker::Alpha::Opaque,
                    );
                });
        });
    editor_theme::request_display_repaint(ui);
    let commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
    let close = ui.input(|input| input.key_pressed(egui::Key::Escape));
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let clicked_outside = ui.input(|input| input.pointer.primary_clicked())
        && !pointer
            .is_some_and(|point| rect.contains(point) || popup.response.rect.contains(point));
    if commit || clicked_outside {
        if let Ok(mut editor) = state.params().editor_state.lock() {
            editor.set_group_name(group_id.get(), &draft);
        }
        crate::editor_shell::request_structural_commit(ui);
    }
    if !close && !commit && !clicked_outside {
        ui.data_mut(|data| data.insert_temp(rename_id.with("draft"), draft));
    }
    if close || clicked_outside {
        let editor_id = egui::Id::new(("generator-group-editor", group_id.get()));
        ui.data_mut(|data| data.remove::<bool>(editor_id));
    }
    (selected != group_accent).then_some(selected)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_group_identity(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: GroupId,
    group_index: usize,
    can_remove_group: bool,
    modules: &[Module],
    _group_size: egui::Vec2,
    collapsed: bool,
    output: GroupOutput,
    group_accent: egui::Color32,
) -> (egui::Rect, GroupOutputInteraction) {
    let palette = editor_theme::semantic();
    let inset = rect.shrink2(egui::vec2(
        editor_theme::shape::STROKE,
        editor_theme::shape::STROKE,
    ));
    let default_group_label = format!("Group {}", group_index + 1);
    let group_label = state
        .params()
        .editor_state
        .lock()
        .ok()
        .and_then(|editor| editor.group_name(group_id.get()).map(str::to_owned))
        .unwrap_or_else(|| default_group_label.clone());
    let painted_group_label = group_label.to_uppercase();
    let action_count = if can_remove_group { 3.0 } else { 2.0 };
    let identity_width = (inset.width() * 0.34)
        .clamp(
            editor_theme::title_height(ui) * 10.0,
            editor_theme::title_height(ui) * 15.0,
        )
        .min(inset.width() * 0.48);
    let action_cell = (inset.height() * 0.72).min(identity_width / (action_count + 1.35));
    let identity = egui::Rect::from_min_size(inset.min, egui::vec2(identity_width, inset.height()));
    let identity_ink = editor_theme::on_accent(group_accent);
    let shoulder = identity.height() * 0.82;
    let mut tab_shape = vec![identity.left_top()];
    for step in 0..=12 {
        let t = step as f32 / 12.0;
        let eased = t * t * (3.0 - 2.0 * t);
        tab_shape.push(egui::pos2(
            identity.right() + shoulder * (1.0 - eased),
            egui::lerp(identity.top()..=identity.bottom(), t),
        ));
    }
    tab_shape.push(identity.left_bottom());
    ui.painter().add(egui::Shape::convex_polygon(
        tab_shape,
        group_accent,
        egui::Stroke::NONE,
    ));
    let controls = egui::Rect::from_min_max(
        egui::pos2(identity.right() + shoulder * 0.88, inset.top()),
        inset.max,
    );
    let remove_width = if can_remove_group { action_cell } else { 0.0 };
    let collapse_rect =
        egui::Rect::from_min_size(identity.min, egui::vec2(action_cell, identity.height()));
    let power_rect = egui::Rect::from_min_max(
        egui::pos2(collapse_rect.right(), identity.top()),
        egui::pos2(collapse_rect.right() + action_cell, identity.bottom()),
    );
    let label_width = ui
        .painter()
        .layout_no_wrap(
            painted_group_label.clone(),
            editor_theme::font::title(),
            identity_ink,
        )
        .size()
        .x
        + editor_theme::space::XS * 2.0;
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(power_rect.right(), identity.top()),
        egui::pos2(
            (power_rect.right() + label_width)
                .min(identity.right() - remove_width - editor_theme::title_height(ui) * 2.2),
            identity.bottom(),
        ),
    );
    let preview_rect = egui::Rect::from_min_max(
        egui::pos2(label_rect.right(), identity.top()),
        egui::pos2(identity.right() - remove_width, identity.bottom()),
    );
    let remove_rect = egui::Rect::from_min_max(
        egui::pos2(preview_rect.right(), identity.top()),
        identity.max,
    );
    let collapse_response = ui
        .interact(
            collapse_rect,
            egui::Id::new(("generator-group-collapse", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if collapsed {
            "Expand this group"
        } else {
            "Collapse this group"
        });
    if collapse_response.has_focus() {
        ui.painter().rect_stroke(
            collapse_rect,
            editor_theme::shape::CONTROL_RADIUS,
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, identity_ink),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().rect_filled(
        collapse_rect.shrink(editor_theme::space::XS),
        editor_theme::shape::CONTROL_RADIUS,
        palette.masthead_ink,
    );
    let group_drag = ui
        .interact(
            label_rect,
            egui::Id::new(("generator-group-drag", group_id.get())),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text("Drag to move; click to rename and recolor; hold Ctrl to duplicate");
    let reorder = if group_drag.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                group_drag.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        ui.input(|input| {
            i8::from(input.key_pressed(egui::Key::ArrowDown))
                - i8::from(input.key_pressed(egui::Key::ArrowUp))
        })
    } else {
        0
    };
    group_drag.dnd_set_drag_payload(group_id);
    let editor_id = egui::Id::new(("generator-group-editor", group_id.get()));
    if group_drag.clicked() && !group_drag.double_clicked() {
        ui.data_mut(|data| {
            let open = data.get_temp::<bool>(editor_id).unwrap_or(false);
            data.insert_temp(editor_id, !open);
            if !open {
                data.insert_temp(
                    egui::Id::new(("generator-group-rename", group_id.get())).with("focus"),
                    true,
                );
            }
        });
    }
    if group_drag.dragged() {
        ui.ctx()
            .set_cursor_icon(if ui.input(|input| input.modifiers.ctrl) {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::Grabbing
            });
    }
    paint_group_cycle(
        ui,
        state,
        preview_rect,
        group_id,
        modules,
        output,
        identity_ink,
    );
    if icon_font_ready(ui) {
        ui.painter().text(
            collapse_rect.center(),
            egui::Align2::CENTER_CENTER,
            if collapsed {
                egui_phosphor::regular::FOLDER
            } else {
                egui_phosphor::regular::FOLDER_OPEN
            },
            editor_theme::font::title(),
            group_accent,
        );
    }
    let label_font = fit_font_to_width(
        ui.painter(),
        &painted_group_label,
        editor_theme::font::title(),
        label_rect.width() * 0.92,
    );
    let label_origin = label_rect.left_center() + egui::vec2(editor_theme::space::XS, 0.0);
    ui.painter().text(
        label_origin,
        egui::Align2::LEFT_CENTER,
        &painted_group_label,
        label_font,
        identity_ink,
    );

    let mut updated_output = output;
    super::draw_envelope_power(ui, power_rect, group_id, &mut updated_output, identity_ink);

    let remove_confirm_id = egui::Id::new(("generator-group-remove-confirm", group_id.get()));
    let mut remove_armed = !modules.is_empty()
        && ui
            .data(|data| data.get_temp::<bool>(remove_confirm_id))
            .unwrap_or(false);
    let remove_response = can_remove_group.then(|| {
        ui.interact(
            remove_rect,
            egui::Id::new(("generator-group-remove", group_id.get())),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if remove_armed {
            "Click again to remove this group and its modules"
        } else {
            "Remove this group and its modules"
        })
    });
    let toggle_collapse = collapse_response.clicked() || keyboard_activate(ui, &collapse_response);
    let mut remove = false;
    if let Some(response) = &remove_response {
        let activate = response.clicked() || keyboard_activate(ui, response);
        if modules.is_empty() {
            remove = activate;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if remove_armed && activate {
            remove = true;
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        } else if activate {
            remove_armed = true;
            ui.data_mut(|data| data.insert_temp(remove_confirm_id, true));
        } else if remove_armed
            && (ui.input(|input| input.key_pressed(egui::Key::Escape))
                || ui.input(|input| {
                    input.pointer.primary_clicked()
                        && input
                            .pointer
                            .latest_pos()
                            .is_some_and(|pointer| !response.rect.contains(pointer))
                }))
        {
            remove_armed = false;
            ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
        }
        let pressed = response.is_pointer_button_down_on();
        if remove_armed || pressed {
            ui.painter().rect_filled(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                translucent(palette.danger, if pressed { 64 } else { 48 }),
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                remove_rect,
                editor_theme::shape::CONTROL_RADIUS,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, palette.danger),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            remove_rect.center(),
            egui::Align2::CENTER_CENTER,
            if icon_font_ready(ui) {
                egui_phosphor::regular::X
            } else {
                ""
            },
            editor_theme::font::label(),
            if remove_armed || pressed || response.hovered() {
                identity_ink
            } else {
                identity_ink.gamma_multiply(0.62)
            },
        );
    }
    if remove_response.is_none() {
        ui.data_mut(|data| data.remove::<bool>(remove_confirm_id));
    }

    (
        controls,
        GroupOutputInteraction {
            remove,
            toggle_collapse,
            reorder,
            accent: None,
            output: (updated_output != output).then_some(updated_output),
        },
    )
}

#[derive(Clone)]
struct GroupCycleCache {
    key: egui::Id,
    points: std::sync::Arc<Vec<egui::Pos2>>,
}

#[derive(Clone, Copy)]
struct GroupPreviewRoute {
    target: usize,
    source: usize,
    control: OscillatorControl,
    amount: f32,
}

fn paint_group_cycle(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    rect: egui::Rect,
    group_id: GroupId,
    modules: &[Module],
    output: GroupOutput,
    ink: egui::Color32,
) {
    if !rect.is_positive() {
        return;
    }
    let response = ui
        .interact(
            rect,
            egui::Id::new(("group-cycle-preview", group_id.get())),
            egui::Sense::hover(),
        )
        .on_hover_text(
            "One cycle of the summed group signal after oscillator modulation and filters",
        );
    let routes = modules
        .iter()
        .filter_map(|module| Some((module.id(), module.oscillator_slot()?)))
        .flat_map(|(module_id, slot)| {
            crate::editor_modulation::generator_preview_routes(ui, state, module_id, slot)
                .into_iter()
                .map(move |(source, control, amount)| GroupPreviewRoute {
                    target: slot.index(),
                    source: usize::from(source),
                    control,
                    amount,
                })
        })
        .collect::<Vec<_>>();
    let count = ((rect.width() * ui.ctx().pixels_per_point()).ceil() as usize).clamp(48, 192);
    let key = group_cycle_key(state, modules, &routes, output, rect, count);
    let cache_id = response.id.with("points");
    let points = ui
        .data(|data| data.get_temp::<GroupCycleCache>(cache_id))
        .filter(|cached| cached.key == key)
        .unwrap_or_else(|| {
            let points = std::sync::Arc::new(group_cycle_points(
                state, modules, &routes, output, rect, count,
            ));
            let cached = GroupCycleCache { key, points };
            ui.data_mut(|data| data.insert_temp(cache_id, cached.clone()));
            cached
        })
        .points;
    let graph = rect.shrink2(egui::vec2(editor_theme::space::XS, rect.height() * 0.20));
    crate::editor_widgets::gradient_area_to_bottom(
        ui.painter(),
        points.as_ref(),
        graph.center().y,
        ink,
        64,
    );
    ui.painter().add(egui::Shape::line(
        points.as_ref().clone(),
        egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, ink),
    ));
}

fn group_cycle_key(
    state: &PluginContext<KurvParams>,
    modules: &[Module],
    routes: &[GroupPreviewRoute],
    output: GroupOutput,
    rect: egui::Rect,
    count: usize,
) -> egui::Id {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    count.hash(&mut hash);
    rect.min.x.to_bits().hash(&mut hash);
    rect.min.y.to_bits().hash(&mut hash);
    rect.width().to_bits().hash(&mut hash);
    rect.height().to_bits().hash(&mut hash);
    output.gain.to_bits().hash(&mut hash);
    for module in modules {
        module.id().get().hash(&mut hash);
        match module.kind() {
            ModuleKind::Oscillator(slot) => {
                let config = state.generator_stack.oscillator_config(slot);
                for value in [
                    config.shape,
                    config.custom_shape,
                    config.pulse_width,
                    config.tuned_frequency_hz(440.0),
                    config.level,
                    config.phase_position,
                    config.phase_warp_amount,
                ] {
                    value.to_bits().hash(&mut hash);
                }
                config.enabled.hash(&mut hash);
                (config.engine as u8).hash(&mut hash);
                config.phase_warp_mode.hash(&mut hash);
                if config.engine.uses_sample_asset()
                    && let Some(summary) = state
                        .resynth_assets
                        .slot(slot.index())
                        .and_then(crate::resynth_state::ResynthSlotState::source_summary)
                {
                    summary.sounding_revision.hash(&mut hash);
                    summary.controls.position.to_bits().hash(&mut hash);
                }
            }
            ModuleKind::Filter(slot) => {
                let config = state.generator_stack.filter_config(slot);
                (config.mode as u8).hash(&mut hash);
                for value in [
                    config.cutoff_hz,
                    config.q,
                    config.slope_db_oct,
                    config.morph,
                    config.shape,
                ] {
                    value.to_bits().hash(&mut hash);
                }
            }
            ModuleKind::Aux(_) => {}
        }
    }
    for route in routes {
        route.target.hash(&mut hash);
        route.source.hash(&mut hash);
        (route.control as u8).hash(&mut hash);
        route.amount.to_bits().hash(&mut hash);
        if let Some(slot) = crate::generators::OscillatorSlot::from_index(route.source) {
            let config = state.generator_stack.oscillator_config(slot);
            for value in [
                config.shape,
                config.pulse_width,
                config.tuned_frequency_hz(440.0),
                config.level,
                config.phase_position,
                config.phase_warp_amount,
            ] {
                value.to_bits().hash(&mut hash);
            }
            config.enabled.hash(&mut hash);
            (config.engine as u8).hash(&mut hash);
            config.phase_warp_mode.hash(&mut hash);
        }
    }
    egui::Id::new(hash.finish())
}

#[allow(clippy::cast_precision_loss, reason = "bounded editor preview indices")]
fn group_cycle_points(
    state: &PluginContext<KurvParams>,
    modules: &[Module],
    routes: &[GroupPreviewRoute],
    output: GroupOutput,
    rect: egui::Rect,
    count: usize,
) -> Vec<egui::Pos2> {
    const SAMPLE_RATE: f32 = 48_000.0;
    let mut filters = Vec::new();
    let mut filter_at = [None; MAX_GENERATOR_MODULES];
    for (module_index, module) in modules.iter().enumerate() {
        if let ModuleKind::Filter(slot) = module.kind() {
            let config = state.generator_stack.filter_config(slot);
            if config.mode != crate::filters::FilterMode::RatioBrickwall {
                filter_at[module_index] = Some(filters.len());
                filters.push((config.coefficients(SAMPLE_RATE), StereoTptSvf::default()));
            }
        }
    }
    let mut bands = [None; MAX_OSCILLATORS];
    for (module_index, module) in modules.iter().enumerate() {
        let ModuleKind::Oscillator(slot) = module.kind() else {
            continue;
        };
        let mut band: Option<(f32, f32)> = None;
        for downstream in &modules[module_index + 1..] {
            let ModuleKind::Filter(filter_slot) = downstream.kind() else {
                continue;
            };
            let config = state.generator_stack.filter_config(filter_slot);
            if config.mode != crate::filters::FilterMode::RatioBrickwall
                || ratio_brickwall_bypassed(config.cutoff_hz, config.shape >= 0.5)
            {
                continue;
            }
            let range = band.get_or_insert((MIN_RATIO, MAX_RATIO));
            if config.shape >= 0.5 {
                range.1 = range.1.min(config.cutoff_hz);
            } else {
                range.0 = range.0.max(config.cutoff_hz);
            }
        }
        bands[slot.index()] = band;
    }
    let mut noise = [crate::oscillators::NoiseState::default(); MAX_OSCILLATORS];
    for (slot, source) in noise.iter_mut().enumerate() {
        source.reset(0x4e4f_4953_455f_5549 ^ slot as u64);
    }
    let resynth = std::array::from_fn::<_, MAX_OSCILLATORS, _>(|slot| {
        let source = state.resynth_assets.slot(slot)?;
        Some((source.sounding_artifact()?, source.rt_grain_controls()?))
    });
    let mut phases = [0.0_f32; MAX_OSCILLATORS];
    let mut samples = Vec::with_capacity(count + 1);
    for frame in 0..count * 5 {
        if frame == count * 4 {
            phases.fill(0.0);
        }
        let mut sample = 0.0;
        let mut oscillator_outputs = [None; MAX_OSCILLATORS];
        for (module_index, module) in modules.iter().enumerate() {
            match module.kind() {
                ModuleKind::Oscillator(slot) => {
                    let config = state.generator_stack.oscillator_config(slot);
                    if !config.enabled {
                        continue;
                    }
                    let mut shape = config.shape;
                    let mut pulse_width = config.pulse_width;
                    let base_pitch = config.tuned_frequency_hz(440.0) / 440.0;
                    let mut pitch_semitones = 0.0;
                    let mut level = config.level;
                    let mut phase_position = config.phase_position;
                    let mut warp = config.phase_warp_amount;
                    let mut pan = config.pan;
                    let mut ring_gain = 1.0;
                    for route in routes.iter().filter(|route| route.target == slot.index()) {
                        let source = oscillator_outputs[route.source].unwrap_or_else(|| {
                            let Some(source_slot) =
                                crate::generators::OscillatorSlot::from_index(route.source)
                            else {
                                return 0.0;
                            };
                            let source_config =
                                state.generator_stack.oscillator_config(source_slot);
                            if !source_config.enabled {
                                return 0.0;
                            }
                            let source_pitch = source_config.tuned_frequency_hz(440.0) / 440.0;
                            (match source_config.engine {
                                OscillatorEngineKind::Va => VaOscillator::preview_shape_ratio(
                                    source_config.shape,
                                    frame as f32 * source_pitch / count as f32
                                        + source_config.phase_position,
                                    source_pitch / count as f32,
                                    source_config.pulse_width,
                                    PhaseWarpMode::from_index(source_config.phase_warp_mode),
                                    source_config.phase_warp_amount,
                                    bands[route.source].unwrap_or((MIN_RATIO, MAX_RATIO)),
                                ),
                                OscillatorEngineKind::Noise => {
                                    let texture = (source_config.pulse_width - 0.03) / 0.94;
                                    let (left, right) = noise[route.source].next(
                                        440.0 / SAMPLE_RATE,
                                        source_config.shape / 3.0,
                                        texture,
                                        source_config.phase_warp_amount,
                                        1,
                                        &[1.0],
                                        &[1.0],
                                    );
                                    (left + right) * 0.5
                                }
                                OscillatorEngineKind::Resynth | OscillatorEngineKind::Grain => {
                                    resynth[route.source].as_ref().map_or(
                                        0.0,
                                        |(artifact, controls)| {
                                            artifact.preview_cycle_sample(
                                                controls.position,
                                                frame as f32 * source_pitch / count as f32,
                                            )
                                        },
                                    )
                                }
                            }) * source_config.level
                        });
                        let value = source * route.amount;
                        match route.control {
                            OscillatorControl::Shape => shape += value * 3.0,
                            OscillatorControl::PulseWidth => pulse_width += value * 0.47,
                            OscillatorControl::Transpose => pitch_semitones += value * 48.0,
                            OscillatorControl::Cents => pitch_semitones += value,
                            OscillatorControl::Level => level += value,
                            OscillatorControl::Pan => pan += value,
                            OscillatorControl::PhasePosition => phase_position += value,
                            OscillatorControl::PhaseWarpAmount => warp += value,
                            OscillatorControl::RingModAmount => {
                                let wet = route.amount.abs();
                                ring_gain *= (1.0 - wet) + source * route.amount.signum() * wet;
                            }
                            _ => {}
                        }
                    }
                    let pitch = base_pitch * 2.0_f32.powf(pitch_semitones / 12.0);
                    let oscillator_sample = match config.engine {
                        OscillatorEngineKind::Va => VaOscillator::preview_shape_ratio(
                            shape.clamp(0.0, 3.0),
                            phases[slot.index()] + phase_position,
                            pitch / count as f32,
                            pulse_width.clamp(0.03, 0.97),
                            PhaseWarpMode::from_index(config.phase_warp_mode),
                            warp.clamp(0.0, 1.0),
                            bands[slot.index()].unwrap_or((MIN_RATIO, MAX_RATIO)),
                        ),
                        OscillatorEngineKind::Noise => {
                            let texture = (config.pulse_width - 0.03) / 0.94;
                            let (left, right) = noise[slot.index()].next(
                                440.0 / SAMPLE_RATE,
                                config.shape / 3.0,
                                texture,
                                config.phase_warp_amount,
                                1,
                                &[1.0],
                                &[1.0],
                            );
                            (left + right) * 0.5
                        }
                        OscillatorEngineKind::Resynth | OscillatorEngineKind::Grain => resynth
                            [slot.index()]
                        .as_ref()
                        .map_or(0.0, |(artifact, controls)| {
                            artifact.preview_cycle_sample(controls.position, phases[slot.index()])
                        }),
                    } * level.clamp(0.0, 1.0)
                        * ring_gain;
                    phases[slot.index()] = (phases[slot.index()] + pitch / count as f32).fract();
                    oscillator_outputs[slot.index()] = Some(oscillator_sample);
                    let pan = pan.clamp(-1.0, 1.0);
                    sample += oscillator_sample * ((1.0 - pan).sqrt() + (1.0 + pan).sqrt()) * 0.5;
                }
                ModuleKind::Filter(_) => {
                    if let Some(index) = filter_at[module_index] {
                        let (coefficients, filter) = &mut filters[index];
                        (sample, _) = filter.process(*coefficients, sample, sample);
                    }
                }
                ModuleKind::Aux(_) => {}
            }
        }
        if frame >= count * 4 {
            samples.push(sample * output.gain);
        }
    }
    samples.push(samples.first().copied().unwrap_or_default());
    let source_count = modules
        .iter()
        .filter(|module| matches!(module.kind(), ModuleKind::Oscillator(_)))
        .count()
        .max(1) as f32;
    let graph = rect.shrink2(egui::vec2(editor_theme::space::XS, rect.height() * 0.20));
    samples
        .into_iter()
        .enumerate()
        .map(|(index, sample)| {
            egui::pos2(
                egui::lerp(graph.left()..=graph.right(), index as f32 / count as f32),
                graph.center().y
                    - (sample / source_count.sqrt()).clamp(-1.2, 1.2) * graph.height() * 0.38,
            )
        })
        .collect()
}

fn keyboard_activate(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.has_focus()
        && ui.input(|input| {
            input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
        })
}
