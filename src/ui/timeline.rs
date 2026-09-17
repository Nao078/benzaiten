use crate::domain::lyrics::LyricLine;
use eframe::egui;

pub const MIN_ZOOM: f32 = 12.0;
pub const MAX_ZOOM: f32 = 600.0;

#[derive(Debug, Clone, Copy)]
pub struct DragAnchor {
    pub index: usize,
    pub start_ms: u64,
    pub length_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub enum TimelineAction {
    Select(usize),
    Seek(u64),
    BeginDrag(usize),
    Drag { index: usize, delta_ms: i64 },
    EndDrag,
}

pub fn show(
    ui: &mut egui::Ui,
    lyrics: &[LyricLine],
    position_ms: u64,
    duration_ms: Option<u64>,
    selected: Option<usize>,
    zoom: &mut f32,
    drag_anchor: Option<DragAnchor>,
) -> Vec<TimelineAction> {
    const LABEL_WIDTH: f32 = 92.0;
    const HEADER_HEIGHT: f32 = 30.0;
    const TRACK_HEIGHT: f32 = 38.0;
    const BLOCK_MARGIN: f32 = 5.0;
    let height = HEADER_HEIGHT + TRACK_HEIGHT * 2.0;
    let inferred_end = lyrics
        .iter()
        .flat_map(|line| [line.start_ms, line.end_ms])
        .flatten()
        .max()
        .unwrap_or(0)
        .saturating_add(5_000);
    let duration = duration_ms.unwrap_or(inferred_end).max(10_000);
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.allocate_ui(egui::vec2(LABEL_WIDTH, height), |ui| {
            ui.add_space(HEADER_HEIGHT + 7.0);
            ui.label("原文");
            ui.add_space(TRACK_HEIGHT - 18.0);
            ui.label("カタカナ");
        });
        egui::ScrollArea::horizontal()
            .id_salt("lyrics-timeline-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ((duration as f32 / 1000.0) * *zoom)
                    .max(ui.available_width())
                    .max(400.0);
                let (canvas, canvas_response) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());

                if ui.rect_contains_pointer(canvas) {
                    let wheel = ui.ctx().input(|input| {
                        if input.modifiers.ctrl {
                            input.raw_scroll_delta.y
                        } else {
                            0.0
                        }
                    });
                    if wheel != 0.0 {
                        *zoom = (*zoom * (wheel * 0.0025).exp()).clamp(MIN_ZOOM, MAX_ZOOM);
                    }
                }

                let painter = ui.painter_at(canvas);
                painter.rect_filled(canvas, 0.0, egui::Color32::from_rgb(25, 28, 32));
                for track in 0..=2 {
                    let y = canvas.top() + HEADER_HEIGHT + TRACK_HEIGHT * track as f32;
                    painter.line_segment(
                        [egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y)],
                        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(70)),
                    );
                }

                let tick_seconds = if *zoom >= 300.0 {
                    0.5
                } else if *zoom >= 100.0 {
                    1.0
                } else if *zoom >= 35.0 {
                    5.0
                } else {
                    10.0
                };
                let tick_count = (duration as f32 / 1000.0 / tick_seconds).ceil() as usize;
                for tick in 0..=tick_count {
                    let seconds = tick as f32 * tick_seconds;
                    let x = canvas.left() + seconds * *zoom;
                    painter.line_segment(
                        [
                            egui::pos2(x, canvas.top() + 17.0),
                            egui::pos2(x, canvas.bottom()),
                        ],
                        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(48)),
                    );
                    painter.text(
                        egui::pos2(x + 3.0, canvas.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        format_ruler_time((seconds * 1000.0) as u64),
                        egui::FontId::monospace(10.0),
                        egui::Color32::LIGHT_GRAY,
                    );
                }

                for (index, line) in lyrics.iter().enumerate() {
                    let Some(start) = line.start_ms else { continue };
                    let end = line.end_ms.or_else(|| {
                        lyrics[index + 1..]
                            .iter()
                            .find_map(|following| following.start_ms)
                    });
                    let display_end = end.unwrap_or(start.saturating_add(2_000));
                    let left = canvas.left() + start as f32 / 1000.0 * *zoom;
                    let right =
                        (canvas.left() + display_end as f32 / 1000.0 * *zoom).max(left + 24.0);
                    for track in 0..2 {
                        if track == 1 && line.reading_text.is_none() {
                            continue;
                        }
                        let top = canvas.top()
                            + HEADER_HEIGHT
                            + track as f32 * TRACK_HEIGHT
                            + BLOCK_MARGIN;
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(left, top),
                            egui::pos2(right, top + TRACK_HEIGHT - BLOCK_MARGIN * 2.0),
                        );
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("timeline-block", track, index)),
                            egui::Sense::click_and_drag(),
                        );
                        let active = selected == Some(index);
                        let color = if active {
                            egui::Color32::from_rgb(49, 112, 196)
                        } else if track == 0 {
                            egui::Color32::from_rgb(43, 75, 145)
                        } else {
                            egui::Color32::from_rgb(94, 58, 141)
                        };
                        painter.rect_filled(rect, 3.0, color);
                        painter.rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(
                                if active { 2.0_f32 } else { 1.0_f32 },
                                egui::Color32::from_gray(180),
                            ),
                            egui::StrokeKind::Inside,
                        );
                        let text = if track == 0 {
                            &line.original_text
                        } else {
                            line.reading_text.as_deref().unwrap_or_default()
                        };
                        painter.text(
                            rect.left_center() + egui::vec2(5.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        );
                        if response.clicked() {
                            actions.push(TimelineAction::Select(index));
                        }
                        if response.double_clicked() {
                            actions.push(TimelineAction::Seek(start));
                        }
                        if response.drag_started() {
                            actions.push(TimelineAction::BeginDrag(index));
                        }
                        if response.dragged()
                            && drag_anchor.is_some_and(|anchor| anchor.index == index)
                        {
                            let delta_ms =
                                (response.drag_delta().x / *zoom * 1000.0).round() as i64;
                            actions.push(TimelineAction::Drag { index, delta_ms });
                        }
                        if response.drag_stopped() {
                            actions.push(TimelineAction::EndDrag);
                        }
                    }
                }

                let playhead_x = canvas.left() + position_ms as f32 / 1000.0 * *zoom;
                painter.line_segment(
                    [
                        egui::pos2(playhead_x, canvas.top()),
                        egui::pos2(playhead_x, canvas.bottom()),
                    ],
                    egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(235, 82, 82)),
                );
                if canvas_response.clicked() {
                    if let Some(pointer) = canvas_response.interact_pointer_pos() {
                        let milliseconds =
                            ((pointer.x - canvas.left()).max(0.0) / *zoom * 1000.0) as u64;
                        actions.push(TimelineAction::Seek(milliseconds.min(duration)));
                    }
                }
            });
    });
    actions
}

fn format_ruler_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}
