//! Final-pass modulation drag feedback, drop targeting, and route inspection.

mod source_drag;

use super::*;
use source_drag::{paint_source_drag_feedback, update_drop_targets};

fn modulation_source_label(
    state: &PluginContext<KurvParams>,
    source: ResolvedRouteSource,
) -> String {
    let ResolvedRouteSource::Rack(source) = source else {
        return "MOD WHEEL".to_owned();
    };
    let index = usize::from(source);
    let envelope = if index < 8 {
        let kind = match index {
            0 => P::Source1Envelope,
            1 => P::Source2Envelope,
            2 => P::Source3Envelope,
            3 => P::Source4Envelope,
            4 => P::Source5Envelope,
            5 => P::Source6Envelope,
            6 => P::Source7Envelope,
            _ => P::Source8Envelope,
        };
        state.get_param(kind) >= 0.5
    } else {
        state.params().modulator_rack.config(index).kind == SourceKind::Envelope
    };
    format!("{} {}", if envelope { "ENV" } else { "LFO" }, index + 1)
}

fn clamp_overlay_rect(rect: egui::Rect, bounds: egui::Rect) -> egui::Rect {
    let max_x = (bounds.right() - rect.width()).max(bounds.left());
    let max_y = (bounds.bottom() - rect.height()).max(bounds.top());
    egui::Rect::from_min_size(
        egui::pos2(
            rect.left().clamp(bounds.left(), max_x),
            rect.top().clamp(bounds.top(), max_y),
        ),
        rect.size(),
    )
}

fn cubic_bezier_points(
    start: egui::Pos2,
    control_a: egui::Pos2,
    control_b: egui::Pos2,
    end: egui::Pos2,
    segments: usize,
) -> Vec<egui::Pos2> {
    (0..=segments)
        .map(|index| {
            let t = index as f32 / segments.max(1) as f32;
            let inverse = 1.0 - t;
            let point = start.to_vec2() * inverse.powi(3)
                + control_a.to_vec2() * (3.0 * inverse.powi(2) * t)
                + control_b.to_vec2() * (3.0 * inverse * t.powi(2))
                + end.to_vec2() * t.powi(3);
            egui::pos2(point.x, point.y)
        })
        .collect()
}

/// Paints the source-hover route editor after every destination has registered
/// its current frame geometry. Destination controls keep their own base-value
/// hit testing; this final pass owns the modulation handles and popup.
pub(crate) fn cancel_interaction(ui: &egui::Ui, state: &PluginContext<KurvParams>) {
    let primary_down = ui.input(|input| input.pointer.primary_down());
    ui.data_mut(|data| {
        let direct =
            data.get_temp_mut_or_default::<DirectModulationState>(egui::Id::new(UI_STATE_ID));
        finish_amount_drag(state, direct, true);
        clear_source_interaction(direct);
        direct.source_drag_cancelled_until_release = primary_down;
    });
}

pub(crate) fn draw_overlay(ui: &mut egui::Ui, state: &PluginContext<KurvParams>) {
    let id = egui::Id::new(UI_STATE_ID);
    let frame = ui.ctx().cumulative_frame_nr();
    let (focused, escape_pressed, primary_down, released, pointer) = ui.input(|input| {
        (
            input.focused,
            input.key_pressed(egui::Key::Escape),
            input.pointer.primary_down(),
            input.pointer.button_released(egui::PointerButton::Primary),
            input.pointer.latest_pos(),
        )
    });
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        prepare_target_frame(direct, frame);
        if !primary_down {
            direct.source_drag_cancelled_until_release = false;
        }
        if direct.amount_drag.is_some() && (escape_pressed || !focused || !primary_down) {
            finish_amount_drag(state, direct, escape_pressed || !focused);
        }
        if direct.dragging_source.is_some() && (escape_pressed || !focused) {
            clear_source_interaction(direct);
            direct.source_drag_cancelled_until_release = primary_down;
        }
        // Once armed, a source drag owns its origin until release. Scrolling or
        // an insertion row can legitimately cull/move the source card mid-drag;
        // treating that as a stale hover cancelled otherwise valid drops.
        if direct.source_rect_frame != frame
            && direct.amount_drag.is_none()
            && direct.dragging_source.is_none()
        {
            clear_source_interaction(direct);
        }
    });
    let mut direct = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .snapshot()
    });
    let mut drag_destinations = None;
    if direct.dragging_source.is_some() {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("kurv-modulation-targets"),
        ));
        let availability = RouteAssignmentSnapshot::capture(
            ui,
            state,
            direct
                .dragging_source
                .expect("dragging source checked above"),
        );
        let bank_full = availability.bank_full();
        drag_destinations = Some(*availability.destinations());
        let hovered_valid = ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            update_drop_targets(&availability, direct, frame, pointer, &painter)
        });
        if let Some(valid) = hovered_valid {
            ui.ctx().set_cursor_icon(if valid {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::NotAllowed
            });
        }
        direct = ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .snapshot()
        });
        paint_source_drag_feedback(ui, state, direct, bank_full);
        if escape_pressed || released || !primary_down {
            let assignment = ui.data_mut(|data| {
                let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
                let assignment = if released
                    && !escape_pressed
                    && direct.hovered_target_valid
                    && pointer.is_some_and(|pointer| direct.hovered_rect.contains(pointer))
                {
                    direct.dragging_source.zip(direct.hovered_target)
                } else {
                    None
                };
                clear_source_interaction(direct);
                assignment
            });
            if let Some((source, target)) = assignment {
                match target {
                    UiDestination::Host(target) => {
                        assign_route(state, source, target);
                    }
                    UiDestination::Modular(target) => {
                        assign_modular_route(state, source, target);
                    }
                }
            }
            direct = ui.data_mut(|data| {
                data.get_temp_mut_or_default::<DirectModulationState>(id)
                    .snapshot()
            });
        }
    }
    if direct.dragging_source.is_none() {
        register_route_handle_widgets(ui, state, drag_destinations.as_ref());
    }
    if direct.dragging_source.is_some() || direct.hovered_source.is_none() {
        clear_inspector_rect(ui, id);
        return;
    }
    let source = direct.hovered_source.expect("hovered source checked above");
    let routes = routes_for_source(ui, state, source);
    if routes.is_empty() {
        clear_inspector_rect(ui, id);
        return;
    }
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let dragging_source_route = direct
        .amount_drag
        .is_some_and(|drag| route_source(state, drag.route) == Some(source));
    if !dragging_source_route
        && !pointer.is_some_and(|pointer| {
            direct.source_rect.contains(pointer) || direct.inspector_rect.contains(pointer)
        })
    {
        ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            direct.hovered_source = None;
            direct.source_rect = egui::Rect::NOTHING;
            direct.inspector_rect = egui::Rect::NOTHING;
        });
        return;
    }

    let title_height = editor_theme::title_height(ui);
    let row_height = title_height * 0.88;
    let inset = editor_theme::space::XS;
    let compact_gap = editor_theme::compact_gap(ui);
    let screen = ui.ctx().content_rect().shrink(editor_theme::space::XXS);
    let width = (direct.source_rect.width() * 1.12)
        .clamp(title_height * 7.6, title_height * 10.0)
        .min(screen.width());
    let header_height = title_height * 0.72;
    let ideal_rows_height =
        routes.len() as f32 * row_height + routes.len().saturating_sub(1) as f32 * compact_gap;
    let rows_height = ideal_rows_height
        .min((screen.height() - inset * 2.0 - header_height - compact_gap).max(0.0));
    let height = inset * 2.0 + header_height + compact_gap + rows_height;
    let below =
        egui::Rect::from_min_size(direct.source_rect.left_bottom(), egui::vec2(width, height));
    let mut popup_rect =
        if below.bottom() <= screen.bottom() || direct.source_rect.top() - height < screen.top() {
            below
        } else {
            egui::Rect::from_min_size(
                egui::pos2(direct.source_rect.left(), direct.source_rect.top() - height),
                egui::vec2(width, height),
            )
        };
    popup_rect = clamp_overlay_rect(popup_rect, screen);

    let mut hovered_link = None;
    let color = modulation_source_color(source);
    let destinations = route_destinations(ui, state);
    let output = egui::Area::new(egui::Id::new("kurv-source-routes"))
        .order(egui::Order::Foreground)
        .fixed_pos(popup_rect.min)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .fill(editor_theme::semantic().chrome)
                .stroke(egui::Stroke::new(
                    editor_theme::shape::STROKE,
                    editor_theme::semantic().grid,
                ))
                .inner_margin(egui::Margin::same(inset.round() as i8))
                .show(ui, |ui| {
                    ui.set_width(width - inset * 2.0);
                    ui.spacing_mut().item_spacing =
                        egui::vec2(editor_theme::space::XXS, compact_gap);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(modulation_source_label(state, source))
                                .font(editor_theme::font::label())
                                .color(color),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}",
                                    routes.len(),
                                    if routes.len() == 1 { "ROUTE" } else { "ROUTES" }
                                ))
                                .font(editor_theme::font::caption())
                                .color(editor_theme::semantic().text_muted),
                            );
                        });
                    });
                    egui::ScrollArea::vertical()
                        .id_salt(("kurv-source-routes-scroll", source))
                        .auto_shrink([false, false])
                        .max_height(rows_height)
                        .show(ui, |ui| {
                            ui.set_width(width - inset * 2.0);
                            for &(route, _, _, _) in routes.as_slice() {
                                let Some(target) = destinations[route] else {
                                    continue;
                                };
                                let active =
                                    direct.amount_drag.is_some_and(|drag| drag.route == route);
                                let row = ui.allocate_ui_with_layout(
                                    egui::vec2(width - inset * 2.0, row_height),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let knob = route_depth_knob(ui, state, route, color);
                                        let amount = route_amount(state, route);
                                        ui.label(
                                            egui::RichText::new(target_label(target))
                                                .font(editor_theme::font::label())
                                                .color(if active {
                                                    color
                                                } else {
                                                    editor_theme::semantic().text
                                                }),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "{:+.0}%",
                                                        amount * 100.0
                                                    ))
                                                    .font(editor_theme::font::value())
                                                    .color(if active {
                                                        color
                                                    } else {
                                                        editor_theme::semantic().text_muted
                                                    }),
                                                );
                                            },
                                        );
                                        knob
                                    },
                                );
                                let hovered =
                                    row.response.contains_pointer() || row.inner.hovered();
                                row.response.context_menu(|ui| {
                                    ui.spacing_mut().item_spacing.y = editor_theme::compact_gap(ui);
                                    if ui.button("Remove route").clicked() {
                                        clear_route(state, route);
                                        ui.close();
                                    }
                                });
                                if hovered || active {
                                    hovered_link =
                                        Some((row.response.rect.center(), target, route));
                                }
                                if hovered || active {
                                    ui.painter().rect_stroke(
                                        row.response.rect.shrink(editor_theme::shape::STROKE * 0.5),
                                        editor_theme::shape::CONTROL_RADIUS,
                                        egui::Stroke::new(
                                            if active {
                                                editor_theme::shape::FOCUS_STROKE
                                            } else {
                                                editor_theme::shape::STROKE
                                            },
                                            color.gamma_multiply(if active { 0.74 } else { 0.34 }),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                            }
                        });
                });
        });
    let rect = output.response.rect;
    ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        direct.inspector_rect = rect;
    });

    if let Some((start, target, route)) = hovered_link {
        let (destination, handle) = ui.data_mut(|data| {
            let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
            let destination = destination_rect(direct, target);
            let handle = (direct.route_handle_mask & (1_u64 << route) != 0)
                .then_some(direct.route_handle_positions[route]);
            (destination, handle)
        });
        if destination.is_positive() {
            let end = handle.unwrap_or_else(|| destination.center());
            let horizontal_span = (end.x - start.x).abs().max(title_height) * 0.35;
            let horizontal_direction = if end.x >= start.x { 1.0 } else { -1.0 };
            let control_a = start + egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let control_b = end - egui::vec2(horizontal_direction * horizontal_span, 0.0);
            let path = cubic_bezier_points(start, control_a, control_b, end, 24);
            ui.painter().add(egui::Shape::dashed_line(
                &path,
                egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, color),
                editor_theme::space::XS,
                editor_theme::space::XXS,
            ));
            brighten_control(ui, destination, color, 30);
        }
    }
}

fn register_route_handle_widgets(
    ui: &egui::Ui,
    state: &PluginContext<KurvParams>,
    drag_destinations: Option<&[Option<UiDestination>; ROUTE_COUNT]>,
) {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    let (route_handle_mask, route_handle_positions, amount_drag) = ui.data_mut(|data| {
        let direct = data.get_temp_mut_or_default::<DirectModulationState>(id);
        (
            direct.route_handle_mask,
            direct.route_handle_positions,
            direct.amount_drag,
        )
    });
    if route_handle_mask == 0 {
        return;
    }
    let owned_destinations;
    let destinations = if let Some(destinations) = drag_destinations {
        destinations
    } else {
        owned_destinations = route_destinations(ui, state);
        &owned_destinations
    };
    for route in 0..ROUTE_COUNT {
        if route_handle_mask & (1_u64 << route) == 0 {
            continue;
        }
        let route_dragging = amount_drag.is_some_and(|drag| drag.route == route);
        let mut response = ui.interact(
            route_handle_hit_rect(route_handle_positions[route], unit),
            route_handle_id(route),
            egui::Sense::click_and_drag(),
        );
        if response.hovered() {
            response = response.on_hover_text(format!(
                "{} · {:+.0}% depth · drag to adjust · double-click to clear",
                destinations[route]
                    .map(target_label)
                    .unwrap_or_else(|| "DESTINATION".to_owned()),
                route_amount(state, route) * 100.0
            ));
        }
        if response.hovered() || route_dragging {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
    }
}

fn clear_inspector_rect(ui: &egui::Ui, id: egui::Id) {
    ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .inspector_rect = egui::Rect::NOTHING;
    });
}

fn destination_rect(direct: &DirectModulationState, target: UiDestination) -> egui::Rect {
    match target {
        UiDestination::Host(target) => direct.target_rects[usize::from(target.saturating_sub(1))],
        UiDestination::Modular(target) => direct.modular_target_rects[..direct.modular_target_len]
            .iter()
            .find(|entry| entry.target == Some(target))
            .map_or(egui::Rect::NOTHING, |entry| entry.rect),
    }
}

fn route_depth_knob(
    ui: &mut egui::Ui,
    state: &PluginContext<KurvParams>,
    route: usize,
    color: egui::Color32,
) -> egui::Response {
    let id = egui::Id::new(UI_STATE_ID);
    let unit = modulation_unit(ui);
    let side = editor_theme::title_height(ui) * 0.82;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag horizontally or vertically to set depth; double-click clears");
    if response.double_clicked() {
        ui.data_mut(|data| {
            finish_amount_drag(
                state,
                data.get_temp_mut_or_default::<DirectModulationState>(id),
                true,
            );
        });
        clear_route(state, route);
    } else if response.drag_started() {
        ui.data_mut(|data| {
            finish_amount_drag(
                state,
                data.get_temp_mut_or_default::<DirectModulationState>(id),
                false,
            );
        });
        let amount = route_amount(state, route);
        begin_route_amount_edit(state, route);
        ui.data_mut(|data| {
            data.get_temp_mut_or_default::<DirectModulationState>(id)
                .amount_drag = Some(AmountDrag {
                route,
                amount,
                initial_amount: amount,
            });
        });
    }
    let route_dragging = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .is_some_and(|drag| drag.route == route)
    });
    if route_dragging {
        if response.dragged() {
            ui.data_mut(|data| {
                let drag = data
                    .get_temp_mut_or_default::<DirectModulationState>(id)
                    .amount_drag
                    .as_mut()
                    .expect("route drag checked above");
                update_route_amount(state, &response, drag);
            });
            editor_theme::request_display_repaint(ui);
        }
        if response.drag_stopped() {
            ui.data_mut(|data| {
                finish_amount_drag(
                    state,
                    data.get_temp_mut_or_default::<DirectModulationState>(id),
                    false,
                );
            });
        }
    }
    let route_dragging = ui.data_mut(|data| {
        data.get_temp_mut_or_default::<DirectModulationState>(id)
            .amount_drag
            .is_some_and(|drag| drag.route == route)
    });
    let amount = route_amount(state, route);
    paint_modulation_knob(
        ui.painter(),
        rect.center(),
        color,
        amount,
        unit,
        response.hovered(),
        response.dragged() || route_dragging,
    );
    response
}
