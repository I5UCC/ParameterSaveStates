//! Short-lived messages shown in the corner (outcomes of use cases).

use std::time::{Duration, Instant};

use egui::Align2;
use psstates_service::NoticeLevel;

use super::*;
use crate::widgets::{GREEN, RED};

pub(super) struct Toast {
    pub(super) level: NoticeLevel,
    pub(super) text: String,
    pub(super) until: Instant,
}

impl MainUi {
    pub(super) fn collect_notices(&mut self) {
        let now = Instant::now();
        for n in self.service.notices_since(&mut self.notice_cursor) {
            let secs = if n.level == NoticeLevel::Error { 8 } else { 4 };
            self.toasts.push(Toast {
                level: n.level,
                text: n.text,
                until: now + Duration::from_secs(secs),
            });
        }
        self.toasts.retain(|t| t.until > now);
        if self.toasts.len() > 5 {
            let excess = self.toasts.len() - 5;
            self.toasts.drain(..excess);
        }
    }

    pub(super) fn toasts_ui(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in &self.toasts {
                    let color = match t.level {
                        NoticeLevel::Error => RED,
                        NoticeLevel::Success => GREEN,
                        NoticeLevel::Info => ui.visuals().text_color(),
                    };
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.colored_label(color, &t.text);
                    });
                    ui.add_space(4.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}
