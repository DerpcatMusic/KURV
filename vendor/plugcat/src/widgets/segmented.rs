use crate::{WidgetTokens, chrome, tokens::LIGHT_TOKENS};
use egui::{Align2, CursorIcon, Rect, Response, Sense, Ui, Vec2};

pub fn segmented(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    labels: &[&str],
    selected: &mut usize,
) -> Response {
    segmented_with_tokens(ui, id_source, labels, selected, &LIGHT_TOKENS)
}

pub fn segmented_with_tokens(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    labels: &[&str],
    selected: &mut usize,
    tokens: &WidgetTokens,
) -> Response {
    let width = labels
        .iter()
        .map(|label| label.chars().count() as f32 * tokens.spacing.sm + tokens.spacing.lg * 1.75)
        .sum::<f32>()
        .max(tokens.spacing.lg * 7.5);
    let height = tokens.spacing.lg + tokens.spacing.md;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    segmented_rect_with_tokens(ui, id_source, rect, labels, selected, tokens)
}

pub fn segmented_rect_with_tokens(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    labels: &[&str],
    selected: &mut usize,
    tokens: &WidgetTokens,
) -> Response {
    let current = *selected;
    segmented_rect_custom_with_tokens(
        ui,
        id_source,
        rect,
        labels.len(),
        Some(current),
        tokens,
        |_, painter, segment, index, active, response| {
            if response.clicked() {
                *selected = index;
            }
            painter.text(
                segment.center(),
                Align2::CENTER_CENTER,
                labels[index],
                chrome::segment_label_font(segment.width() < 28.0),
                chrome::segment_text_color(tokens, active),
            );
        },
    )
}

pub fn segmented_rect_custom_with_tokens(
    ui: &mut Ui,
    id_source: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    segments: usize,
    selected: Option<usize>,
    tokens: &WidgetTokens,
    mut paint_segment: impl FnMut(&Ui, &egui::Painter, Rect, usize, bool, &Response),
) -> Response {
    assert!(segments > 0, "segmented control needs at least one segment");

    let painter = ui.painter_at(rect);
    chrome::draw_group_shell(&painter, rect, segments, tokens);

    let base_id = ui.make_persistent_id(id_source);
    let mut response: Option<Response> = None;
    for index in 0..segments {
        let segment = chrome::segment_rect(rect, segments, index);
        let segment_response = ui
            .interact(segment, base_id.with(index), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let active = selected == Some(index);
        if active {
            chrome::draw_segment_pressed(&painter, segment, index, segments, tokens);
        } else if segment_response.hovered() {
            chrome::draw_segment_hover(&painter, segment, index, segments, tokens);
        }
        paint_segment(ui, &painter, segment, index, active, &segment_response);
        response = Some(match response {
            Some(response) => response.union(segment_response),
            None => segment_response,
        });
    }

    response.expect("segmented control needs at least one segment")
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect};

    #[test]
    fn segment_rects_cover_parent_without_gaps() {
        let rect = Rect::from_min_max(Pos2::new(3.0, 7.0), Pos2::new(104.0, 29.0));
        let segments = 4;
        let rects = (0..segments)
            .map(|index| chrome::segment_rect(rect, segments, index))
            .collect::<Vec<_>>();

        assert_eq!(rects.first().map(Rect::left), Some(rect.left()));
        assert_eq!(rects.last().map(Rect::right), Some(rect.right()));

        for pair in rects.windows(2) {
            let [left, right] = pair else {
                continue;
            };
            assert_eq!(left.right(), right.left());
            assert_eq!(left.top(), rect.top());
            assert_eq!(left.bottom(), rect.bottom());
            assert_eq!(right.top(), rect.top());
            assert_eq!(right.bottom(), rect.bottom());
        }

        let expected_width = rect.width() / segments as f32;
        for segment in rects {
            assert!((segment.width() - expected_width).abs() <= f32::EPSILON);
        }
    }
}
