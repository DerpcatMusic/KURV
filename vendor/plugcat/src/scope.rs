use egui::{Color32, Pos2, Rect, Stroke, Ui, pos2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScopePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ScopeScale {
    Linear,
    #[default]
    Log,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopePolarity {
    #[default]
    Bipolar,
    Unipolar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeRender {
    #[default]
    Line,
    Fill,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeView {
    pub start: f32,
    pub end: f32,
    pub vertical_zoom: f32,
}

impl Default for ScopeView {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 1.0,
            vertical_zoom: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScopeState {
    pub points: Vec<ScopePoint>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeOptions {
    pub view: ScopeView,
    pub scale: ScopeScale,
    pub polarity: ScopePolarity,
    pub render: ScopeRender,
}

impl Default for ScopeOptions {
    fn default() -> Self {
        Self {
            view: ScopeView::default(),
            scale: ScopeScale::default(),
            polarity: ScopePolarity::default(),
            render: ScopeRender::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeStyle {
    pub stroke: Stroke,
    pub fill: Color32,
    pub zero: Stroke,
}

impl Default for ScopeStyle {
    fn default() -> Self {
        Self {
            stroke: Stroke::new(1.4_f32, Color32::from_rgb(92, 202, 216)),
            fill: Color32::from_rgba_unmultiplied(92, 202, 216, 38),
            zero: Stroke::new(1.0_f32, Color32::from_white_alpha(42)),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScopeScratch {
    points: Vec<Pos2>,
}

pub fn paint_egui_scope(
    ui: &mut Ui,
    rect: Rect,
    points: &[ScopePoint],
    options: ScopeOptions,
    style: ScopeStyle,
    scratch: &mut ScopeScratch,
) {
    scratch.points.clear();
    if points.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    let zero_y = match options.polarity {
        ScopePolarity::Bipolar => rect.center().y,
        ScopePolarity::Unipolar => rect.bottom(),
    };
    ui.painter().line_segment(
        [pos2(rect.left(), zero_y), pos2(rect.right(), zero_y)],
        style.zero,
    );

    for point in points {
        let x = rect.left() + point.x.clamp(0.0, 1.0) * rect.width();
        let y_norm = match options.polarity {
            ScopePolarity::Bipolar => (point.y * options.view.vertical_zoom).clamp(-1.0, 1.0),
            ScopePolarity::Unipolar => (point.y * options.view.vertical_zoom).clamp(0.0, 1.0),
        };
        let y = match options.polarity {
            ScopePolarity::Bipolar => rect.center().y - y_norm * rect.height() * 0.5,
            ScopePolarity::Unipolar => rect.bottom() - y_norm * rect.height(),
        };
        scratch.points.push(pos2(x, y));
    }

    if scratch.points.len() >= 2 {
        ui.painter()
            .add(egui::Shape::line(scratch.points.clone(), style.stroke));
    }
}
