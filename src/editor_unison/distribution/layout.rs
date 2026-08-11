//! Distribution panel geometry.

use crate::editor_theme;

pub(super) fn compact_unison_layout(
    rect: egui::Rect,
) -> (egui::Rect, egui::Rect, egui::Rect, egui::Rect) {
    let inset = editor_theme::space::XXS.min(rect.width().min(rect.height()) * 0.035);
    let content = rect.shrink(inset);
    let header_height =
        (editor_theme::font::LABEL_SIZE + editor_theme::space::XXS * 2.0).min(content.height());
    let header = egui::Rect::from_min_size(content.min, egui::vec2(content.width(), header_height));
    let view = content;
    let rail_width =
        (editor_theme::font::CAPTION_SIZE + editor_theme::space::XS).min(content.width());
    let rail = egui::Rect::from_min_max(
        egui::pos2(
            (view.right() - rail_width).max(view.left()),
            header.bottom(),
        ),
        view.max,
    );
    let plot = egui::Rect::from_min_max(
        egui::pos2(view.left(), header.bottom()),
        egui::pos2(
            (rail.left() - editor_theme::space::XXS).max(view.left()),
            view.bottom(),
        ),
    );
    (header, view, plot, rail)
}
