use std::hash::Hash;

use egui::{
    Color32, CursorIcon, Id, InnerResponse, Rect, Response, Sense, Stroke, Ui, UiBuilder, Vec2,
    ViewportCommand, pos2, vec2,
};

use super::{FloatingWindowSizing, ResizeBounds, ResizeDrag, ResizeEdge, UiZoom, WindowScaleSpace};

#[derive(Clone, Debug)]
pub struct ResizeAnywhereConfig {
    pub hit_size: f32,
    pub bounds: ResizeBounds,
    pub enabled_edges: Vec<ResizeEdge>,
    pub show_corner_marks: bool,
    pub aspect_ratio: Option<(u32, u32)>,
    /// Converts pointer movement in egui content points into the host-facing
    /// logical size space. This is the user UI zoom, not native DPI.
    pub content_to_outer_scale: f32,
}

impl ResizeAnywhereConfig {
    pub fn new(bounds: ResizeBounds) -> Self {
        Self {
            hit_size: 8.0,
            bounds,
            enabled_edges: ResizeEdge::ALL.to_vec(),
            show_corner_marks: false,
            aspect_ratio: None,
            content_to_outer_scale: 1.0,
        }
    }

    #[must_use]
    pub fn hit_size(mut self, hit_size: f32) -> Self {
        self.hit_size = hit_size.max(1.0);
        self
    }

    #[must_use]
    pub fn enabled_edges(mut self, enabled_edges: impl Into<Vec<ResizeEdge>>) -> Self {
        self.enabled_edges = enabled_edges.into();
        self
    }

    #[must_use]
    pub const fn show_corner_marks(mut self, show_corner_marks: bool) -> Self {
        self.show_corner_marks = show_corner_marks;
        self
    }

    #[must_use]
    pub const fn aspect_ratio(mut self, aspect_ratio: Option<(u32, u32)>) -> Self {
        self.aspect_ratio = aspect_ratio;
        self
    }

    #[must_use]
    pub fn content_to_outer_scale(mut self, scale: f32) -> Self {
        self.content_to_outer_scale = if scale.is_finite() && scale > 0.01 {
            scale
        } else {
            1.0
        };
        self
    }

    /// Set the user zoom used to convert egui pointer movement into host
    /// logical size deltas. Native DPI is deliberately not part of this path.
    #[must_use]
    pub fn ui_zoom(self, zoom: UiZoom) -> Self {
        self.content_to_outer_scale(zoom.get())
    }

    /// Use only the UI-zoom boundary from a scale space. The native DPI
    /// boundary belongs to the renderer and must not affect resize deltas.
    #[must_use]
    pub fn scale_space(self, scale_space: WindowScaleSpace) -> Self {
        self.ui_zoom(scale_space.ui_zoom())
    }
}

impl Default for ResizeAnywhereConfig {
    fn default() -> Self {
        Self::new(ResizeBounds::new(16, 16, 4096, 4096))
    }
}

pub fn resize_anywhere<R>(
    ui: &mut Ui,
    id_source: impl Hash + std::fmt::Debug,
    current_size: (u32, u32),
    config: ResizeAnywhereConfig,
    mut request_resize: impl FnMut((u32, u32)),
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let ui_rect = ui.clip_rect();
    let mut content_ui = ui.new_child(UiBuilder::new().max_rect(ui_rect).layout(*ui.layout()));
    let output = add_contents(&mut content_ui);
    let id = ui.make_persistent_id(id_source);

    for edge in config.enabled_edges.iter().copied() {
        let rect = edge_rect(ui_rect, edge, config.hit_size);
        let response = ui.interact(rect, id.with(edge_id(edge)), Sense::drag());
        let drag_id = id.with(("drag", edge_id(edge)));
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(cursor_for_edge(edge));
        }

        if response.drag_started()
            && let Some(pointer_pos) = response.interact_pointer_pos()
        {
            let drag = ResizeDrag::new(
                edge,
                current_size,
                (pointer_pos.x, pointer_pos.y),
                config.bounds,
            );
            ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
        }

        if response.dragged()
            && let Some(pointer_pos) = response.interact_pointer_pos()
            && let Some(drag) = ui.ctx().data(|data| data.get_temp::<ResizeDrag>(drag_id))
        {
            let mut requested = drag.requested_size_scaled(
                (pointer_pos.x, pointer_pos.y),
                config.content_to_outer_scale,
            );
            if let Some(aspect_ratio) = config.aspect_ratio {
                requested = fit_aspect_ratio(requested, edge, config.bounds, aspect_ratio);
            }
            if requested != current_size {
                request_resize(requested);
            }
        }

        if response.drag_stopped() {
            ui.ctx().data_mut(|data| data.remove::<ResizeDrag>(drag_id));
        }

        if config.show_corner_marks && edge_is_corner(edge) {
            paint_corner_mark(ui, &response, edge);
        }
    }

    InnerResponse {
        inner: output,
        response: ui.response(),
    }
}

fn fit_aspect_ratio(
    size: (u32, u32),
    edge: ResizeEdge,
    bounds: ResizeBounds,
    aspect_ratio: (u32, u32),
) -> (u32, u32) {
    let ratio = aspect_ratio.0.max(1) as f32 / aspect_ratio.1.max(1) as f32;
    let width_from_height = size.1 as f32 * ratio;
    let desired_width = if edge.affects_height() && !edge.affects_width() {
        width_from_height
    } else if edge.affects_width() && edge.affects_height() {
        let height_from_width = size.0 as f32 / ratio;
        if (width_from_height - size.0 as f32).abs() < (height_from_width - size.1 as f32).abs() {
            width_from_height
        } else {
            size.0 as f32
        }
    } else {
        size.0 as f32
    };
    let min_width = (bounds.min_height as f32 * ratio)
        .ceil()
        .max(bounds.min_width as f32);
    let max_width = (bounds.max_height as f32 * ratio)
        .floor()
        .min(bounds.max_width as f32)
        .max(min_width);
    let width = desired_width.clamp(min_width, max_width).round() as u32;
    (width, (width as f32 / ratio).round().max(1.0) as u32)
}

pub fn request_fit_content_for_response(
    ui: &Ui,
    id_source: impl Hash + std::fmt::Debug,
    response: &Response,
    sizing: FloatingWindowSizing,
) {
    let FloatingWindowSizing::FitContent {
        min_width,
        min_height,
        max_width,
        max_height,
        padding_x,
        padding_y,
    } = sizing
    else {
        return;
    };

    request_fit_content_size(
        ui,
        id_source,
        response.rect.size(),
        vec2(min_width as f32, min_height as f32),
        vec2(max_width as f32, max_height as f32),
        vec2(padding_x as f32, padding_y as f32),
    );
}

pub fn request_fit_content_size(
    ui: &Ui,
    id_source: impl Hash + std::fmt::Debug,
    desired_size: Vec2,
    min_size: Vec2,
    max_size: Vec2,
    padding: Vec2,
) {
    let size = (desired_size + padding).max(min_size).min(max_size);
    let id = ui.make_persistent_id(id_source);
    let epsilon = (2.0 / ui.ctx().pixels_per_point()).max(1.0);
    let viewport_size = ui.ctx().input(|input| input.content_rect().size());
    let viewport_delta = viewport_size - size;
    let viewport_mismatched = viewport_delta.x.abs() > epsilon || viewport_delta.y.abs() > epsilon;

    if viewport_mismatched || should_request_resize(ui, id, size, epsilon) {
        ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(size));
    }
}

fn should_request_resize(ui: &Ui, id: Id, size: Vec2, epsilon: f32) -> bool {
    ui.ctx().data_mut(|data| {
        if let Some(previous) = data.get_temp::<Vec2>(id) {
            let delta = previous - size;
            if delta.x.abs() <= epsilon && delta.y.abs() <= epsilon {
                return false;
            }
        }

        data.insert_temp(id, size);
        true
    })
}

fn edge_rect(rect: Rect, edge: ResizeEdge, hit_size: f32) -> Rect {
    let hit = hit_size.max(1.0);
    match edge {
        ResizeEdge::North => Rect::from_min_max(
            pos2(rect.left() + hit, rect.top()),
            pos2(rect.right() - hit, rect.top() + hit),
        ),
        ResizeEdge::South => Rect::from_min_max(
            pos2(rect.left() + hit, rect.bottom() - hit),
            pos2(rect.right() - hit, rect.bottom()),
        ),
        ResizeEdge::East => Rect::from_min_max(
            pos2(rect.right() - hit, rect.top() + hit),
            pos2(rect.right(), rect.bottom() - hit),
        ),
        ResizeEdge::West => Rect::from_min_max(
            pos2(rect.left(), rect.top() + hit),
            pos2(rect.left() + hit, rect.bottom() - hit),
        ),
        ResizeEdge::NorthEast => Rect::from_min_max(
            pos2(rect.right() - hit, rect.top()),
            pos2(rect.right(), rect.top() + hit),
        ),
        ResizeEdge::NorthWest => {
            Rect::from_min_max(rect.left_top(), rect.left_top() + Vec2::splat(hit))
        }
        ResizeEdge::SouthEast => {
            Rect::from_min_max(rect.right_bottom() - Vec2::splat(hit), rect.right_bottom())
        }
        ResizeEdge::SouthWest => Rect::from_min_max(
            pos2(rect.left(), rect.bottom() - hit),
            pos2(rect.left() + hit, rect.bottom()),
        ),
    }
}

fn cursor_for_edge(edge: ResizeEdge) -> CursorIcon {
    match edge {
        ResizeEdge::East => CursorIcon::ResizeEast,
        ResizeEdge::North => CursorIcon::ResizeNorth,
        ResizeEdge::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeEdge::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeEdge::South => CursorIcon::ResizeSouth,
        ResizeEdge::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeEdge::SouthWest => CursorIcon::ResizeSouthWest,
        ResizeEdge::West => CursorIcon::ResizeWest,
    }
}

fn edge_id(edge: ResizeEdge) -> &'static str {
    match edge {
        ResizeEdge::East => "east",
        ResizeEdge::North => "north",
        ResizeEdge::NorthEast => "north-east",
        ResizeEdge::NorthWest => "north-west",
        ResizeEdge::South => "south",
        ResizeEdge::SouthEast => "south-east",
        ResizeEdge::SouthWest => "south-west",
        ResizeEdge::West => "west",
    }
}

fn edge_is_corner(edge: ResizeEdge) -> bool {
    matches!(
        edge,
        ResizeEdge::NorthEast
            | ResizeEdge::NorthWest
            | ResizeEdge::SouthEast
            | ResizeEdge::SouthWest
    )
}

fn paint_corner_mark(ui: &Ui, response: &Response, edge: ResizeEdge) {
    let stroke = if response.hovered() || response.dragged() {
        ui.style().interact(response).fg_stroke
    } else {
        Stroke::new(1.0_f32, Color32::from_white_alpha(44))
    };
    let rect = response.rect.shrink(2.0);
    let painter = ui.painter();
    let lines = match edge {
        ResizeEdge::NorthEast => [
            [
                pos2(rect.right() - 8.0, rect.top()),
                pos2(rect.right(), rect.top()),
            ],
            [
                pos2(rect.right(), rect.top()),
                pos2(rect.right(), rect.top() + 8.0),
            ],
        ],
        ResizeEdge::NorthWest => [
            [
                pos2(rect.left(), rect.top()),
                pos2(rect.left() + 8.0, rect.top()),
            ],
            [
                pos2(rect.left(), rect.top()),
                pos2(rect.left(), rect.top() + 8.0),
            ],
        ],
        ResizeEdge::SouthEast => [
            [
                pos2(rect.right() - 8.0, rect.bottom()),
                pos2(rect.right(), rect.bottom()),
            ],
            [
                pos2(rect.right(), rect.bottom() - 8.0),
                pos2(rect.right(), rect.bottom()),
            ],
        ],
        ResizeEdge::SouthWest => [
            [
                pos2(rect.left(), rect.bottom()),
                pos2(rect.left() + 8.0, rect.bottom()),
            ],
            [
                pos2(rect.left(), rect.bottom() - 8.0),
                pos2(rect.left(), rect.bottom()),
            ],
        ],
        _ => return,
    };

    for line in lines {
        painter.line_segment(line, stroke);
    }
}
