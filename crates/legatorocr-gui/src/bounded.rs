//! Keeps every window inside the screen: it never grows past the viewport, its content scrolls
//! instead, and the title bar (with the close button) therefore always stays reachable.

pub trait Bounded {
    fn bounded(self, ctx: &eframe::egui::Context) -> Self;
}

impl Bounded for eframe::egui::Window<'_> {
    fn bounded(self, ctx: &eframe::egui::Context) -> Self {
        let r = ctx.content_rect();
        let (mw, mh) = (
            (r.width() - 16.0).max(240.0),
            (r.height() - 40.0).max(240.0),
        );
        self.constrain(true)
            .max_width(mw)
            .max_height(mh)
            .scroll([true, true])
    }
}

#[cfg(test)]
mod tests {
    use super::Bounded;
    use eframe::egui;

    #[test]
    fn tall_window_stays_on_screen() {
        for (w, h) in [
            (1920.0, 1080.0),
            (1366.0, 768.0),
            (1024.0, 600.0),
            (800.0, 480.0),
            (390.0, 740.0),
        ] {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
            for _ in 0..3 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = ui.ctx().clone();
                        egui::Window::new("t")
                            .bounded(&ctx)
                            .default_size([1100.0, 900.0])
                            .min_size([640.0, 560.0])
                            .show(&ctx, |ui| {
                                for i in 0..200 {
                                    ui.label(format!(
                                        "line {i} with some long text to widen the window"
                                    ));
                                }
                            });
                    },
                );
                out.textures_delta.clear();
            }
            let id = ctx.memory(|m| {
                m.areas()
                    .visible_layer_ids()
                    .into_iter()
                    .find(|l| l.order == egui::Order::Middle)
            });
            let r = ctx.memory(|m| m.area_rect(id.unwrap().id)).unwrap();
            assert!(
                r.min.x >= -1.0 && r.min.y >= -1.0 && r.max.x <= w + 1.0 && r.max.y <= h + 1.0,
                "{w}x{h}: {r:?}"
            );
        }
    }
}
