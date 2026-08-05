#[cfg(feature = "egui")]
pub mod egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowBackendKind {
    BaseviewParented,
    BaseviewWindowed,
    WinitWindowed,
}

/// Host-provided renderer scale. This is native DPI, not editor zoom.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct NativeDpi(f32);

impl NativeDpi {
    pub fn new(value: f32) -> Self {
        Self(valid_scale(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for NativeDpi {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// User-selected editor zoom. Ordinary host resizing must never modify this.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct UiZoom(f32);

impl UiZoom {
    pub fn new(value: f32) -> Self {
        Self(valid_scale(value))
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

impl Default for UiZoom {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// The size reported by the host in logical points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostLogicalSize {
    pub width: u32,
    pub height: u32,
}

impl HostLogicalSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn from_tuple(size: (u32, u32)) -> Self {
        Self::new(size.0, size.1)
    }

    pub const fn as_tuple(self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn scaled(self, factor: f32) -> Self {
        Self::from_tuple(scale_size(self.as_tuple(), valid_scale(factor)))
    }

    pub fn is_within(self, other: Self, tolerance: u32) -> bool {
        self.width.abs_diff(other.width) <= tolerance
            && self.height.abs_diff(other.height) <= tolerance
    }
}

/// Explicit conversions between the three size spaces used by plugin windows.
///
/// UI zoom maps content units to the host-facing outer logical size. Native DPI
/// alone maps that outer logical size to the renderer's physical backing size.
/// Keeping these factors separate prevents user zoom from being applied twice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowScaleSpace {
    native_dpi: NativeDpi,
    ui_zoom: UiZoom,
}

impl WindowScaleSpace {
    pub fn new(native_pixels_per_point: f32, ui_zoom: f32) -> Self {
        Self::from_scales(
            NativeDpi::new(native_pixels_per_point),
            UiZoom::new(ui_zoom),
        )
    }

    pub const fn from_scales(native_dpi: NativeDpi, ui_zoom: UiZoom) -> Self {
        Self {
            native_dpi,
            ui_zoom,
        }
    }

    pub const fn native_dpi(self) -> NativeDpi {
        self.native_dpi
    }

    pub const fn ui_zoom(self) -> UiZoom {
        self.ui_zoom
    }

    pub const fn with_ui_zoom(self, ui_zoom: UiZoom) -> Self {
        Self::from_scales(self.native_dpi, ui_zoom)
    }

    pub fn content_to_outer(self, size: (u32, u32)) -> (u32, u32) {
        scale_size(size, self.ui_zoom.get())
    }

    pub fn outer_to_physical(self, size: (u32, u32)) -> (u32, u32) {
        scale_size(size, self.native_dpi.get())
    }

    pub fn physical_to_outer(self, size: (u32, u32)) -> (u32, u32) {
        scale_size(size, self.native_dpi.get().recip())
    }
}

/// Immutable scale state shared by an editor and its host adapter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowScaleState {
    native_dpi: NativeDpi,
    ui_zoom: UiZoom,
    host_size: HostLogicalSize,
}

impl WindowScaleState {
    pub const fn new(native_dpi: NativeDpi, ui_zoom: UiZoom, host_size: HostLogicalSize) -> Self {
        Self {
            native_dpi,
            ui_zoom,
            host_size,
        }
    }

    pub const fn native_dpi(self) -> NativeDpi {
        self.native_dpi
    }

    pub const fn ui_zoom(self) -> UiZoom {
        self.ui_zoom
    }

    pub const fn host_size(self) -> HostLogicalSize {
        self.host_size
    }

    pub const fn scale_space(self) -> WindowScaleSpace {
        WindowScaleSpace::from_scales(self.native_dpi, self.ui_zoom)
    }

    /// Apply an ordinary host resize. The user zoom is intentionally unchanged.
    pub const fn on_host_resize(self, host_size: HostLogicalSize) -> Self {
        Self::new(self.native_dpi, self.ui_zoom, host_size)
    }

    pub const fn on_native_dpi_change(self, native_dpi: NativeDpi) -> Self {
        Self::new(native_dpi, self.ui_zoom, self.host_size)
    }

    pub fn begin_ui_zoom_change(self, requested: UiZoom) -> ResizeTransaction {
        ResizeTransaction::new(self.ui_zoom, requested, self.host_size)
    }

    fn with_zoom_and_size(self, ui_zoom: UiZoom, host_size: HostLogicalSize) -> Self {
        Self::new(self.native_dpi, ui_zoom, host_size)
    }
}

/// Explicit transaction for a user zoom change.
///
/// A host adapter may request `target_size`, then commit only after the host
/// reports a matching logical size. A normal resize uses `on_host_resize` and
/// never enters this transaction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeTransaction {
    previous_zoom: UiZoom,
    requested_zoom: UiZoom,
    start_size: HostLogicalSize,
    target_size: HostLogicalSize,
}

impl ResizeTransaction {
    pub fn new(previous_zoom: UiZoom, requested_zoom: UiZoom, start_size: HostLogicalSize) -> Self {
        let ratio = requested_zoom.get() / previous_zoom.get();
        Self {
            previous_zoom,
            requested_zoom,
            start_size,
            target_size: start_size.scaled(ratio),
        }
    }

    pub const fn previous_zoom(self) -> UiZoom {
        self.previous_zoom
    }

    pub const fn requested_zoom(self) -> UiZoom {
        self.requested_zoom
    }

    pub const fn start_size(self) -> HostLogicalSize {
        self.start_size
    }

    pub const fn target_size(self) -> HostLogicalSize {
        self.target_size
    }

    pub fn is_confirmed(self, actual_size: HostLogicalSize) -> bool {
        actual_size.is_within(self.target_size, 1)
    }

    pub fn commit(
        self,
        state: WindowScaleState,
        actual_size: HostLogicalSize,
    ) -> Option<WindowScaleState> {
        self.is_confirmed(actual_size)
            .then(|| state.with_zoom_and_size(self.requested_zoom, actual_size))
    }
}

fn valid_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.01 {
        scale
    } else {
        1.0
    }
}

fn scale_size(size: (u32, u32), scale: f32) -> (u32, u32) {
    (
        (size.0 as f32 * scale).round().max(1.0) as u32,
        (size.1 as f32 * scale).round().max(1.0) as u32,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    East,
    North,
    NorthEast,
    NorthWest,
    South,
    SouthEast,
    SouthWest,
    West,
}

impl ResizeEdge {
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::South,
        Self::East,
        Self::West,
        Self::NorthEast,
        Self::NorthWest,
        Self::SouthEast,
        Self::SouthWest,
    ];

    pub const fn affects_width(self) -> bool {
        matches!(
            self,
            Self::East
                | Self::NorthEast
                | Self::NorthWest
                | Self::SouthEast
                | Self::SouthWest
                | Self::West
        )
    }

    pub const fn affects_height(self) -> bool {
        matches!(
            self,
            Self::North
                | Self::NorthEast
                | Self::NorthWest
                | Self::South
                | Self::SouthEast
                | Self::SouthWest
        )
    }

    pub const fn grows_from_west(self) -> bool {
        matches!(self, Self::NorthWest | Self::SouthWest | Self::West)
    }

    pub const fn grows_from_north(self) -> bool {
        matches!(self, Self::North | Self::NorthEast | Self::NorthWest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeBounds {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

impl ResizeBounds {
    pub const fn new(min_width: u32, min_height: u32, max_width: u32, max_height: u32) -> Self {
        Self {
            min_width,
            min_height,
            max_width,
            max_height,
        }
    }

    pub fn clamp(self, width: f32, height: f32) -> (u32, u32) {
        let min_width = self.min_width.max(1) as f32;
        let min_height = self.min_height.max(1) as f32;
        let max_width = self.max_width.max(self.min_width.max(1)) as f32;
        let max_height = self.max_height.max(self.min_height.max(1)) as f32;

        (
            width.clamp(min_width, max_width).round() as u32,
            height.clamp(min_height, max_height).round() as u32,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResizeDrag {
    pub edge: ResizeEdge,
    pub start_size: (u32, u32),
    pub start_cursor: (f32, f32),
    pub bounds: ResizeBounds,
}

impl ResizeDrag {
    pub const fn new(
        edge: ResizeEdge,
        start_size: (u32, u32),
        start_cursor: (f32, f32),
        bounds: ResizeBounds,
    ) -> Self {
        Self {
            edge,
            start_size,
            start_cursor,
            bounds,
        }
    }

    pub fn requested_size(self, cursor: (f32, f32)) -> (u32, u32) {
        self.requested_size_scaled(cursor, 1.0)
    }

    pub fn requested_size_scaled(
        self,
        cursor: (f32, f32),
        content_to_outer_scale: f32,
    ) -> (u32, u32) {
        let scale = valid_scale(content_to_outer_scale);
        let delta_x = (cursor.0 - self.start_cursor.0) * scale;
        let delta_y = (cursor.1 - self.start_cursor.1) * scale;
        let width_delta = if self.edge.affects_width() {
            if self.edge.grows_from_west() {
                -delta_x
            } else {
                delta_x
            }
        } else {
            0.0
        };
        let height_delta = if self.edge.affects_height() {
            if self.edge.grows_from_north() {
                -delta_y
            } else {
                delta_y
            }
        } else {
            0.0
        };

        self.bounds.clamp(
            self.start_size.0 as f32 + width_delta,
            self.start_size.1 as f32 + height_delta,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowResizeStrategy {
    HostManaged,
    NativeDecorations,
    ClientDragResize,
    ProgrammaticDrag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeResizeCapability {
    pub backend: WindowBackendKind,
    pub strategy: WindowResizeStrategy,
    pub edge_drag: bool,
    pub corner_drag: bool,
    pub client_drag_resize: bool,
}

impl NativeResizeCapability {
    pub const fn for_backend(backend: WindowBackendKind) -> Self {
        match backend {
            WindowBackendKind::BaseviewParented => Self {
                backend,
                strategy: WindowResizeStrategy::HostManaged,
                edge_drag: false,
                corner_drag: false,
                client_drag_resize: false,
            },
            WindowBackendKind::BaseviewWindowed => Self {
                backend,
                strategy: WindowResizeStrategy::NativeDecorations,
                edge_drag: true,
                corner_drag: true,
                client_drag_resize: true,
            },
            WindowBackendKind::WinitWindowed => Self {
                backend,
                strategy: WindowResizeStrategy::ClientDragResize,
                edge_drag: true,
                corner_drag: true,
                client_drag_resize: true,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginWindowSpec {
    pub title: &'static str,
    pub initial_size: (u32, u32),
    pub min_size: (u32, u32),
    pub max_size: Option<(u32, u32)>,
    pub user_resizable: bool,
}

impl PluginWindowSpec {
    pub const fn new(title: &'static str, width: u32, height: u32) -> Self {
        Self {
            title,
            initial_size: (width, height),
            min_size: (1, 1),
            max_size: None,
            user_resizable: true,
        }
    }

    pub const fn min_size(mut self, width: u32, height: u32) -> Self {
        self.min_size = (width, height);
        self
    }

    pub const fn max_size(mut self, width: u32, height: u32) -> Self {
        self.max_size = Some((width, height));
        self
    }

    pub const fn user_resizable(mut self, enabled: bool) -> Self {
        self.user_resizable = enabled;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FloatingWindowPlacement {
    #[default]
    Default,
    Cursor {
        offset_x: i32,
        offset_y: i32,
    },
    AnchorRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        offset_x: i32,
        offset_y: i32,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FloatingWindowSizing {
    #[default]
    Fixed,
    FitContent {
        min_width: u32,
        min_height: u32,
        max_width: u32,
        max_height: u32,
        padding_x: u32,
        padding_y: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FloatingWindowSpec {
    pub id: &'static str,
    pub title: String,
    pub initial_size: (u32, u32),
    pub placement: FloatingWindowPlacement,
    pub sizing: FloatingWindowSizing,
    pub behavior: FloatingWindowBehavior,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloatingWindowBehavior {
    pub force_on_top: bool,
    pub close_on_parent_focus: bool,
    pub user_resizable: bool,
}

impl Default for FloatingWindowBehavior {
    fn default() -> Self {
        Self {
            force_on_top: false,
            close_on_parent_focus: false,
            user_resizable: true,
        }
    }
}

impl FloatingWindowSpec {
    pub fn new(id: &'static str, title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            id,
            title: title.into(),
            initial_size: (width, height),
            placement: FloatingWindowPlacement::default(),
            sizing: FloatingWindowSizing::default(),
            behavior: FloatingWindowBehavior::default(),
        }
    }

    pub fn fit_content(
        id: &'static str,
        title: impl Into<String>,
        initial_width: u32,
        initial_height: u32,
        min_width: u32,
        min_height: u32,
        max_width: u32,
        max_height: u32,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            initial_size: (initial_width, initial_height),
            placement: FloatingWindowPlacement::default(),
            sizing: FloatingWindowSizing::FitContent {
                min_width,
                min_height,
                max_width,
                max_height,
                padding_x: 0,
                padding_y: 0,
            },
            behavior: FloatingWindowBehavior::default(),
        }
    }

    pub fn at_cursor(mut self) -> Self {
        self.placement = FloatingWindowPlacement::Cursor {
            offset_x: 0,
            offset_y: 0,
        };
        self
    }

    pub fn with_cursor_offset(mut self, offset_x: i32, offset_y: i32) -> Self {
        self.placement = FloatingWindowPlacement::Cursor { offset_x, offset_y };
        self
    }

    pub fn at_anchor_rect(mut self, rect: FloatingAnchorRect) -> Self {
        self.placement = FloatingWindowPlacement::AnchorRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            offset_x: 0,
            offset_y: 0,
        };
        self
    }

    pub fn with_anchor_offset(mut self, offset_x: i32, offset_y: i32) -> Self {
        if let FloatingWindowPlacement::AnchorRect {
            offset_x: ref mut current_offset_x,
            offset_y: ref mut current_offset_y,
            ..
        } = self.placement
        {
            *current_offset_x = offset_x;
            *current_offset_y = offset_y;
        }
        self
    }

    pub fn with_fit_padding(mut self, padding_x: u32, padding_y: u32) -> Self {
        if let FloatingWindowSizing::FitContent {
            padding_x: ref mut current_padding_x,
            padding_y: ref mut current_padding_y,
            ..
        } = self.sizing
        {
            *current_padding_x = padding_x;
            *current_padding_y = padding_y;
        }
        self
    }

    pub fn force_on_top(mut self, enabled: bool) -> Self {
        self.behavior.force_on_top = enabled;
        self
    }

    pub fn close_on_parent_focus(mut self, enabled: bool) -> Self {
        self.behavior.close_on_parent_focus = enabled;
        self
    }

    pub fn user_resizable(mut self, enabled: bool) -> Self {
        self.behavior.user_resizable = enabled;
        self
    }

    pub fn menu_behavior(self) -> Self {
        self.force_on_top(true).close_on_parent_focus(true)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingAnchorRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl FloatingAnchorRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn top_center(self) -> (f64, f64) {
        ((self.x + self.width * 0.5) as f64, self.y as f64)
    }

    pub fn bottom_center(self) -> (f64, f64) {
        (
            (self.x + self.width * 0.5) as f64,
            (self.y + self.height) as f64,
        )
    }
}

pub fn clamp_floating_size(desired_size: (f32, f32), sizing: FloatingWindowSizing) -> (u32, u32) {
    match sizing {
        FloatingWindowSizing::Fixed => (
            desired_size.0.round().max(1.0) as u32,
            desired_size.1.round().max(1.0) as u32,
        ),
        FloatingWindowSizing::FitContent {
            min_width,
            min_height,
            max_width,
            max_height,
            padding_x,
            padding_y,
        } => {
            let bounds = ResizeBounds::new(min_width, min_height, max_width, max_height);
            bounds.clamp(
                desired_size.0 + padding_x as f32,
                desired_size.1 + padding_y as f32,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_scale_space_applies_native_dpi_and_ui_zoom_on_separate_boundaries() {
        let scale = WindowScaleSpace::new(1.5, 1.5);

        assert_eq!(scale.content_to_outer((1000, 667)), (1500, 1001));
        assert_eq!(scale.outer_to_physical((1500, 1000)), (2250, 1500));
        assert_eq!(scale.physical_to_outer((2250, 1500)), (1500, 1000));
    }

    #[test]
    fn resize_drag_handles_all_edges() {
        let bounds = ResizeBounds::new(50, 40, 300, 240);
        let start = (100, 80);
        let cursor = (10.0, 12.0);

        assert_eq!(
            ResizeDrag::new(ResizeEdge::East, start, cursor, bounds).requested_size((20.0, 99.0)),
            (110, 80)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::West, start, cursor, bounds).requested_size((20.0, 99.0)),
            (90, 80)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::South, start, cursor, bounds).requested_size((99.0, 22.0)),
            (100, 90)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::North, start, cursor, bounds).requested_size((99.0, 22.0)),
            (100, 70)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::SouthEast, start, cursor, bounds)
                .requested_size((20.0, 22.0)),
            (110, 90)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::SouthWest, start, cursor, bounds)
                .requested_size((20.0, 22.0)),
            (90, 90)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::NorthEast, start, cursor, bounds)
                .requested_size((20.0, 22.0)),
            (110, 70)
        );
        assert_eq!(
            ResizeDrag::new(ResizeEdge::NorthWest, start, cursor, bounds)
                .requested_size((20.0, 22.0)),
            (90, 70)
        );
    }

    #[test]
    fn resize_drag_clamps_to_bounds() {
        let bounds = ResizeBounds::new(90, 70, 120, 100);
        let drag = ResizeDrag::new(ResizeEdge::SouthEast, (100, 80), (0.0, 0.0), bounds);

        assert_eq!(drag.requested_size((-50.0, -50.0)), (90, 70));
        assert_eq!(drag.requested_size((200.0, 200.0)), (120, 100));
    }

    #[test]
    fn resize_drag_converts_content_delta_to_outer_window_units() {
        let bounds = ResizeBounds::new(50, 40, 300, 240);
        let drag = ResizeDrag::new(ResizeEdge::SouthEast, (100, 80), (0.0, 0.0), bounds);

        assert_eq!(drag.requested_size_scaled((10.0, 10.0), 1.5), (115, 95));
        assert_eq!(drag.requested_size_scaled((10.0, 10.0), 0.75), (108, 88));
    }
}
