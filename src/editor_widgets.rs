//! Reusable, low-ceremony building blocks for KURV editor shells.

use std::hash::Hash;
use std::sync::Arc;

use crate::editor_theme;

pub(crate) fn paint_vertical_label(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let origin = egui::Align2::CENTER_CENTER
        .anchor_size(rect.center(), galley.size())
        .min;
    ui.painter().add(
        egui::epaint::TextShape::new(origin, galley, color)
            .with_angle_and_anchor(-std::f32::consts::FRAC_PI_2, egui::Align2::CENTER_CENTER),
    );
}

/// Tessellate immutable editor geometry once so continuous gestures can submit
/// an Arc-backed mesh instead of rebuilding point vectors and path triangles on
/// every pointer frame.
pub(crate) fn stroke_mesh(
    ctx: &egui::Context,
    points: Vec<egui::Pos2>,
    stroke: egui::Stroke,
) -> Arc<egui::Mesh> {
    let options = ctx.tessellation_options(Clone::clone);
    let mut tessellator =
        egui::epaint::Tessellator::new(ctx.pixels_per_point(), options, [1, 1], Vec::new());
    let mut mesh = egui::Mesh::default();
    tessellator.tessellate_shape(egui::Shape::line(points, stroke), &mut mesh);
    Arc::new(mesh)
}

#[derive(Clone)]
struct MeshCache {
    key: egui::Id,
    mesh: Arc<egui::Mesh>,
}

fn cached_mesh(
    ui: &egui::Ui,
    cache_id: egui::Id,
    key: impl Hash,
    build: impl FnOnce() -> Arc<egui::Mesh>,
) -> Arc<egui::Mesh> {
    let key = egui::Id::new(key);
    if let Some(cached) = ui
        .data(|store| store.get_temp::<MeshCache>(cache_id))
        .filter(|cached| cached.key == key)
    {
        return cached.mesh;
    }
    let mesh = build();
    ui.data_mut(|store| {
        store.insert_temp(
            cache_id,
            MeshCache {
                key,
                mesh: Arc::clone(&mesh),
            },
        );
    });
    mesh
}

pub(crate) fn cached_stroke_mesh(
    ui: &egui::Ui,
    cache_id: egui::Id,
    key: impl Hash,
    points: impl FnOnce() -> Vec<egui::Pos2>,
    stroke: egui::Stroke,
) -> Arc<egui::Mesh> {
    cached_mesh(ui, cache_id, key, || {
        stroke_mesh(ui.ctx(), points(), stroke)
    })
}

pub(crate) fn cached_gradient_stroke_mesh(
    ui: &egui::Ui,
    cache_id: egui::Id,
    key: impl Hash,
    points: impl FnOnce() -> Vec<egui::Pos2>,
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
    stroke: egui::Stroke,
) -> Arc<egui::Mesh> {
    cached_mesh(ui, cache_id, key, || {
        let points = points();
        let mut mesh = gradient_mesh(&points, baseline, color, edge_alpha);
        let options = ui.ctx().tessellation_options(Clone::clone);
        let mut tessellator = egui::epaint::Tessellator::new(
            ui.ctx().pixels_per_point(),
            options,
            [1, 1],
            Vec::new(),
        );
        tessellator.tessellate_shape(egui::Shape::line(points, stroke), &mut mesh);
        Arc::new(mesh)
    })
}

pub(crate) fn icon_font_ready(_ui: &egui::Ui) -> bool {
    true
}

pub(crate) fn with_dragged_layer<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    active: bool,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if !active {
        return add_contents(ui);
    }

    let layer = egui::LayerId::new(egui::Order::Tooltip, id);
    let result = ui
        .scope_builder(egui::UiBuilder::new().layer_id(layer), add_contents)
        .inner;
    if let Some(delta) = ui.input(|input| {
        input
            .pointer
            .latest_pos()
            .zip(input.pointer.press_origin())
            .map(|(pointer, origin)| pointer - origin)
    }) {
        ui.ctx()
            .transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(delta));
    }
    ui.ctx()
        .set_cursor_icon(if ui.input(|input| input.modifiers.ctrl) {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::Grabbing
        });
    editor_theme::request_display_repaint(ui);
    result
}

pub(crate) fn paint_power_icon(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::POWER,
        egui::FontId::proportional(rect.height() * 0.58),
        color,
    );
}

pub(crate) fn with_child(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id_salt: impl Hash,
    layout: egui::Layout,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(layout),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add_contents(&mut child);
}

pub(crate) fn menu_choice(
    ui: &mut egui::Ui,
    ordinal: usize,
    label: &str,
    enabled: bool,
    width: f32,
    height: f32,
    accent: egui::Color32,
) -> bool {
    let minimum = editor_theme::shape::STROKE;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.max(minimum), height.max(minimum)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response.on_hover_text(format!(
            "{label} is unavailable at the current module limit"
        ))
    };
    let active = enabled
        && (response.hovered() || response.has_focus() || response.is_pointer_button_down_on());
    let palette = editor_theme::semantic();
    let ordinal_text = ordinal.to_string();
    let ordinal_galley = ui.painter().layout_no_wrap(
        ordinal_text,
        editor_theme::font::caption(),
        egui::Color32::PLACEHOLDER,
    );
    let content = rect.shrink2(egui::vec2(editor_theme::space::SM, 0.0));
    let ordinal_rect = egui::Rect::from_min_size(
        content.min,
        egui::vec2(
            ordinal_galley.size().x + editor_theme::space::SM,
            content.height(),
        ),
    );
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(ordinal_rect.right(), content.top()),
        content.right_bottom(),
    );
    if active {
        ui.painter().line_segment(
            [
                egui::pos2(rect.left(), rect.top() + editor_theme::space::XXS),
                egui::pos2(rect.left(), rect.bottom() - editor_theme::space::XXS),
            ],
            egui::Stroke::new(editor_theme::shape::FOCUS_STROKE, accent),
        );
    }
    ui.painter().galley(
        egui::pos2(
            ordinal_rect.left(),
            ordinal_rect.center().y - ordinal_galley.size().y * 0.5,
        ),
        ordinal_galley,
        if enabled {
            accent.gamma_multiply(if active { 1.0 } else { 0.72 })
        } else {
            palette.disabled_text
        },
    );
    ui.painter().text(
        label_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        editor_theme::font::label(),
        if !enabled {
            palette.disabled_text
        } else if active {
            palette.text
        } else {
            palette.text_muted
        },
    );
    let keyboard = enabled
        && response.has_focus()
        && ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Space)
        });
    response.clicked() || keyboard
}

/// Scroll the nearest parent rack while a reorder gesture is held near a
/// viewport edge. Distance into the edge zone controls speed, so crossing a
/// long rack does not require dropping and re-grabbing the module.
pub(crate) fn drag_edge_scroll(ui: &egui::Ui, viewport: egui::Rect, active: bool) {
    if !active {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.latest_pos()) else {
        return;
    };
    let edge = editor_theme::title_height(ui) * 1.5;
    if pointer.x < viewport.left() - edge || pointer.x > viewport.right() + edge {
        return;
    }
    let pressure = if pointer.y < viewport.top() + edge {
        ((viewport.top() + edge - pointer.y) / edge).clamp(0.0, 1.0)
    } else if pointer.y > viewport.bottom() - edge {
        -((pointer.y - (viewport.bottom() - edge)) / edge).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if pressure == 0.0 {
        return;
    }
    let dt = ui.input(|input| input.stable_dt).min(0.1);
    let points_per_second = editor_theme::title_height(ui) * 8.0;
    ui.scroll_with_delta(egui::vec2(0.0, pressure * points_per_second * dt));
    editor_theme::request_display_repaint(ui);
}

pub(crate) fn gradient_area_to_bottom(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    bottom: f32,
    color: egui::Color32,
    top_alpha: u8,
) {
    gradient_area(painter, points, bottom, color, top_alpha);
}

fn gradient_area(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) {
    let mesh = gradient_mesh(points, baseline, color, edge_alpha);
    if !mesh.is_empty() {
        painter.add(mesh);
    }
}

fn gradient_mesh(
    points: &[egui::Pos2],
    baseline: f32,
    color: egui::Color32,
    edge_alpha: u8,
) -> egui::Mesh {
    if points.len() < 2 || edge_alpha == 0 || !baseline.is_finite() {
        return egui::Mesh::default();
    }
    let edge = with_alpha(color, edge_alpha);
    let transparent = with_alpha(color, 0);
    let mut mesh = egui::Mesh::default();
    mesh.reserve_vertices((points.len() - 1) * 4);
    mesh.reserve_triangles((points.len() - 1) * 2);
    for pair in points.windows(2) {
        if pair
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite())
        {
            continue;
        }
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], edge);
        mesh.colored_vertex(pair[1], edge);
        mesh.colored_vertex(egui::pos2(pair[1].x, baseline), transparent);
        mesh.colored_vertex(egui::pos2(pair[0].x, baseline), transparent);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    mesh
}

fn with_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
