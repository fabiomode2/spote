//! Ventana del mini reproductor.
//!
//! La UI se dibuja a mano sobre un panel redondeado: cabecera arrastrable,
//! portada, espectro, progreso y controles geométricos. No usa widgets de
//! formulario salvo las zonas interactivas (`ui.allocate_rect`), así el
//! resultado es igual en X11 y Wayland.

use crate::net::{MiniLink, NetEvent};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use spote_protocol::mini::{
    format_time, mini_socket_path, MiniCommand, MiniSnapshot, MINI_BAND_COUNT,
};

const BG: Color32 = Color32::from_rgb(7, 8, 11);
const PANEL: Color32 = Color32::from_rgb(17, 18, 24);
const PANEL_HI: Color32 = Color32::from_rgb(26, 28, 36);
const TEXT: Color32 = Color32::from_rgb(237, 238, 243);
const MUTED: Color32 = Color32::from_rgb(140, 144, 157);
const FAINT: Color32 = Color32::from_rgb(52, 55, 66);
const DEFAULT_ACCENT: Color32 = Color32::from_rgb(92, 196, 170);

const ART_SIZE: f32 = 68.0;
const CORNER: u8 = 14;
const FRAME_MS: u64 = 33;

pub struct MiniApp {
    link: MiniLink,
    snapshot: MiniSnapshot,
    /// Instante egui del último snapshot, para extrapolar la posición.
    snapshot_at: f64,
    connected: bool,
    quit_sent: bool,

    art_source: Option<String>,
    texture: Option<TextureHandle>,

    smooth: [f32; MINI_BAND_COUNT],
    peaks: [f32; MINI_BAND_COUNT],
    last_frame: f64,

    seek_preview: Option<u64>,
    seek_sent_at: f64,

    /// Estado de play/pausa optimista mientras llega el siguiente snapshot.
    play_hold_until: f64,
    play_local: bool,

    volume_local: f32,
    volume_hold_until: f64,
    volume_sent_at: f64,
}

impl MiniApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(TEXT);
        visuals.window_fill = PANEL;
        visuals.panel_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.window_stroke = Stroke::new(1.0_f32, FAINT);
        cc.egui_ctx.set_visuals(visuals);

        let snapshot = MiniSnapshot::default();
        let volume_local = snapshot.volume;
        Self {
            link: MiniLink::spawn(mini_socket_path()),
            snapshot,
            snapshot_at: 0.0,
            connected: false,
            quit_sent: false,
            art_source: None,
            texture: None,
            smooth: [0.0; MINI_BAND_COUNT],
            peaks: [0.0; MINI_BAND_COUNT],
            last_frame: 0.0,
            seek_preview: None,
            seek_sent_at: 0.0,
            play_hold_until: 0.0,
            play_local: false,
            volume_local,
            volume_hold_until: 0.0,
            volume_sent_at: 0.0,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        while let Some(event) = self.link.try_recv() {
            match event {
                NetEvent::Connected => self.connected = true,
                NetEvent::Disconnected => self.connected = false,
                NetEvent::Quit => {
                    self.quit_sent = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if let Some(snapshot) = self.link.take_snapshot() {
            self.apply_snapshot(snapshot, now);
        }
    }

    fn apply_snapshot(&mut self, snapshot: MiniSnapshot, now: f64) {
        if snapshot.track_key != self.snapshot.track_key {
            self.smooth = [0.0; MINI_BAND_COUNT];
            self.peaks = [0.0; MINI_BAND_COUNT];
            self.seek_preview = None;
            self.play_hold_until = 0.0;
            // La portada puede llegar después del primer snapshot: al cambiar
            // de pista se fuerza a releerla aunque la ruta se repita.
            self.art_source = None;
            self.texture = None;
        }
        self.snapshot_at = now;
        let optimistic_play = now < self.play_hold_until;
        if now >= self.volume_hold_until {
            self.volume_local = clean_volume(snapshot.volume);
        }
        let local_playing = self.play_local;
        self.snapshot = snapshot;
        if optimistic_play {
            self.snapshot.playing = local_playing;
        } else {
            self.play_local = self.snapshot.playing;
        }
    }

    fn ensure_art(&mut self, ctx: &egui::Context) {
        let wanted = self
            .snapshot
            .art_path
            .clone()
            .map(|path| format!("{}|{path}", self.snapshot.track_key));
        if wanted == self.art_source {
            return;
        }
        self.art_source = wanted;
        self.texture = None;

        let Some(path) = self.snapshot.art_path.clone() else {
            return;
        };

        if let Ok(image) = image::open(&path) {
            let image = image.thumbnail(320, 320);
            let rgba = image.to_rgba8();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            self.texture = Some(ctx.load_texture(
                format!("spote-mini-art:{path}"),
                color_image,
                TextureOptions::LINEAR,
            ));
        }
    }

    fn animate(&mut self, now: f64, dt: f32) {
        let targets = self.band_targets(now);
        let step = (dt * 60.0).clamp(0.0, 2.0);
        for i in 0..MINI_BAND_COUNT {
            let target = targets[i];
            let rate = if target > self.smooth[i] { 0.50 } else { 0.16 };
            self.smooth[i] += (target - self.smooth[i]) * (rate * step).clamp(0.0, 1.0);
            if self.smooth[i] >= self.peaks[i] {
                self.peaks[i] = self.smooth[i];
            } else {
                self.peaks[i] = (self.peaks[i] - dt * 0.30).max(self.smooth[i]);
            }
        }
    }

    fn band_targets(&self, now: f64) -> [f32; MINI_BAND_COUNT] {
        let mut targets = [0.0f32; MINI_BAND_COUNT];
        let bands = &self.snapshot.bands;
        let has_signal = bands
            .iter()
            .take(MINI_BAND_COUNT)
            .any(|band| band.is_finite() && *band > 0.005);

        if has_signal {
            if bands.len() >= MINI_BAND_COUNT {
                for (target, band) in targets.iter_mut().zip(bands.iter().take(MINI_BAND_COUNT)) {
                    *target = clean_band(*band);
                }
            } else {
                let last = bands.len().saturating_sub(1);
                for (i, target) in targets.iter_mut().enumerate() {
                    let at = i as f32 * last as f32 / (MINI_BAND_COUNT - 1) as f32;
                    let low = at.floor() as usize;
                    let high = (low + 1).min(last);
                    let mix = at - low as f32;
                    *target = clean_band(bands[low] * (1.0 - mix) + bands[high] * mix);
                }
            }
            return targets;
        }

        // Sin datos de audio: una onda baja y lenta, un poco más viva si suena.
        let time = now as f32;
        let amplitude = if self.snapshot.playing { 0.16 } else { 0.05 };
        for (i, target) in targets.iter_mut().enumerate() {
            let x = i as f32 / (MINI_BAND_COUNT - 1) as f32;
            let wave = (time * 1.7 + i as f32 * 0.55).sin() * 0.5 + 0.5;
            let ripple = (time * 0.9 - i as f32 * 0.31).sin() * 0.5 + 0.5;
            let arch = (x * std::f32::consts::PI).sin();
            *target = 0.015 + amplitude * wave * ripple * (0.35 + 0.65 * arch);
        }
        targets
    }

    fn position_ms(&self, now: f64) -> u64 {
        if let Some(preview) = self.seek_preview {
            return preview;
        }
        let base = self.snapshot.position_ms;
        let position = if self.snapshot.playing && self.connected {
            let elapsed = ((now - self.snapshot_at).max(0.0) * 1000.0) as u64;
            base.saturating_add(elapsed)
        } else {
            base
        };
        if self.snapshot.duration_ms > 0 {
            position.min(self.snapshot.duration_ms)
        } else {
            position
        }
    }

    fn send_seek(&mut self, position_ms: u64, now: f64, force: bool) {
        if force || now - self.seek_sent_at >= 0.10 {
            self.seek_sent_at = now;
            self.link.send(MiniCommand::seek_to(position_ms));
        }
    }

    fn commit_seek(&mut self, position_ms: u64, now: f64) {
        let position_ms = if self.snapshot.duration_ms > 0 {
            position_ms.min(self.snapshot.duration_ms)
        } else {
            position_ms
        };
        self.link.send(MiniCommand::seek_to(position_ms));
        self.seek_preview = None;
        self.snapshot.position_ms = position_ms;
        self.snapshot_at = now;
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        if !self.quit_sent {
            self.quit_sent = true;
            self.link.send(MiniCommand::Quit);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn accent(&self) -> Color32 {
        accent_color(self.snapshot.accent)
    }

    fn paint(&mut self, ui: &mut egui::Ui, now: f64) {
        let accent = self.accent();
        let full = ui.max_rect();
        let panel = full.shrink(6.0);
        let content =
            Rect::from_min_max(panel.min + vec2(12.0, 11.0), panel.max - vec2(12.0, 11.0));
        let painter = ui.painter().with_clip_rect(panel);

        painter.rect_filled(panel, CornerRadius::same(CORNER), PANEL);
        painter.rect_stroke(
            panel,
            CornerRadius::same(CORNER),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
            StrokeKind::Inside,
        );
        // Halo tenue del color de la portada detrás del arte.
        painter.circle_filled(
            pos2(content.left() + ART_SIZE * 0.5, content.top() + 78.0),
            ART_SIZE,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 9),
        );

        self.paint_header(ui, &painter, content, now, accent);
        self.paint_body(ui, &painter, content, now, accent);
        self.paint_progress(ui, &painter, content, now, accent);
        self.paint_controls(ui, &painter, content, now, accent);
    }

    fn paint_header(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        now: f64,
        accent: Color32,
    ) {
        let close = Rect::from_min_size(
            pos2(content.right() - 24.0, content.top() + 1.0),
            Vec2::splat(24.0),
        );
        let dot_center = pos2(close.left() - 15.0, content.top() + 12.0);

        let drag = ui.allocate_rect(
            Rect::from_min_max(
                content.left_top(),
                pos2(close.left() - 4.0, content.top() + 40.0),
            ),
            Sense::click_and_drag(),
        );
        if drag.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let title_width = (dot_center.x - 10.0 - content.left()).max(24.0);
        let title = if self.snapshot.title.is_empty() {
            "Sin reproducción"
        } else {
            self.snapshot.title.as_str()
        };
        paint_text(
            ui,
            pos2(content.left(), content.top()),
            title,
            FontId::proportional(15.0),
            TEXT,
            title_width,
        );

        let subtitle = track_subtitle(&self.snapshot);
        paint_text(
            ui,
            pos2(content.left(), content.top() + 19.0),
            &subtitle,
            FontId::proportional(11.0),
            MUTED,
            title_width,
        );

        painter.line_segment(
            [
                pos2(content.left(), content.top() + 42.0),
                pos2(content.right(), content.top() + 42.0),
            ],
            Stroke::new(1.0_f32, scale(accent, 0.22)),
        );

        let dot = ui.allocate_rect(
            Rect::from_center_size(dot_center, Vec2::splat(14.0)),
            Sense::hover(),
        );
        if self.connected {
            painter.circle_filled(dot_center, 3.5, accent);
            painter.circle_stroke(
                dot_center,
                6.0,
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 60),
                ),
            );
        } else {
            let pulse = (now * 2.0).sin() as f32 * 0.5 + 0.5;
            let muted_red = Color32::from_rgb(152, 96, 96);
            painter.circle_filled(dot_center, 3.0, muted_red);
            painter.circle_stroke(
                dot_center,
                5.0 + pulse * 2.0,
                Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(
                        muted_red.r(),
                        muted_red.g(),
                        muted_red.b(),
                        (20.0 + pulse * 55.0) as u8,
                    ),
                ),
            );
        }
        let tip = if self.connected {
            let status = if self.snapshot.status.is_empty() {
                "conectado"
            } else {
                self.snapshot.status.as_str()
            };
            format!("{status}\n{}", self.link.socket_path.display())
        } else {
            format!(
                "spote no está abierto\nreintentando cada 1 s\n{}",
                self.link.socket_path.display()
            )
        };
        dot.on_hover_text(tip);

        let close_response = ui.allocate_rect(close, Sense::click());
        if close_response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            painter.rect_filled(close, CornerRadius::same(7), PANEL_HI);
        }
        let close_color = if close_response.hovered() {
            TEXT
        } else {
            MUTED
        };
        draw_close(painter, close, close_color);
        if close_response.clicked() {
            self.request_close(ui.ctx());
        }
    }

    fn paint_body(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        _now: f64,
        accent: Color32,
    ) {
        let art = Rect::from_min_size(
            pos2(content.left(), content.top() + 48.0),
            Vec2::splat(ART_SIZE),
        );
        let spectrum = Rect::from_min_max(
            pos2(art.right() + 14.0, art.top()),
            pos2(content.right(), art.bottom()),
        );

        if let Some(texture) = &self.texture {
            ui.put(
                art,
                egui::Image::new(texture)
                    .fit_to_exact_size(art.size())
                    .corner_radius(10.0),
            );
        } else {
            painter.rect_filled(art, CornerRadius::same(10), mix(PANEL, accent, 0.16));
            let center = art.center();
            let radius = art.width() * 0.30;
            painter.circle_filled(center, radius, Color32::from_rgb(11, 12, 16));
            painter.circle_stroke(center, radius, Stroke::new(1.0_f32, scale(accent, 0.65)));
            painter.circle_filled(center, 5.0, accent);
            painter.circle_filled(center, 1.8, Color32::from_rgb(24, 25, 31));
        }
        painter.rect_stroke(
            art,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
            StrokeKind::Inside,
        );

        let has_viz = self.snapshot.viz_rows > 0
            && self.snapshot.viz_cols > 0
            && !self.snapshot.viz.is_empty();
        if has_viz {
            self.paint_viz(ui, painter, spectrum, accent);
        } else {
            self.paint_spectrum(painter, spectrum, accent);
        }
        self.paint_viz_chip(ui, painter, spectrum, accent);

        if !self.connected {
            let overlay = spectrum.shrink(1.0);
            painter.rect_filled(
                overlay,
                CornerRadius::same(10),
                Color32::from_rgba_unmultiplied(8, 9, 12, 226),
            );
            painter.text(
                pos2(spectrum.center().x, spectrum.center().y - 9.0),
                Align2::CENTER_CENTER,
                "spote no está abierto",
                FontId::proportional(12.0),
                TEXT,
            );
            painter.text(
                pos2(spectrum.center().x, spectrum.center().y + 10.0),
                Align2::CENTER_CENTER,
                "reintentando cada 1 s",
                FontId::proportional(10.0),
                MUTED,
            );
        }
    }

    fn paint_viz(
        &self,
        _ui: &mut egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        accent: Color32,
    ) {
        painter.rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(8, 9, 13));
        let cols = self.snapshot.viz_cols.max(1) as f32;
        let rows = self.snapshot.viz_rows.max(1) as f32;
        let cell_w = rect.width() / cols;
        let cell_h = rect.height() / rows;
        let font = FontId::monospace((cell_h * 0.95).min(cell_w * 1.15).max(5.0));
        for cell in &self.snapshot.viz {
            if cell.alpha < 8 || cell.ch == ' ' {
                continue;
            }
            let color = Color32::from_rgba_unmultiplied(
                cell.fg[0],
                cell.fg[1],
                cell.fg[2],
                cell.alpha,
            );
            let pos = pos2(
                rect.left() + cell.x as f32 * cell_w,
                rect.top() + cell.y as f32 * cell_h,
            );
            painter.text(pos, Align2::LEFT_TOP, cell.ch, font.clone(), color);
        }
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 14)),
            StrokeKind::Inside,
        );
        // Mantén el acento visible en el borde inferior, como la franja TUI.
        painter.line_segment(
            [
                pos2(rect.left() + 1.0, rect.bottom() - 1.0),
                pos2(rect.right() - 1.0, rect.bottom() - 1.0),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90)),
        );
    }

    fn paint_viz_chip(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        accent: Color32,
    ) {
        let label = if self.snapshot.viz_preset.is_empty() {
            "viz".to_string()
        } else {
            format!("viz · {}", self.snapshot.viz_preset)
        };
        let width = (label.chars().count() as f32 * 6.2 + 12.0).min(rect.width() - 8.0);
        let chip = Rect::from_min_size(
            pos2(rect.left() + 4.0, rect.top() + 4.0),
            vec2(width.max(30.0), 16.0),
        );
        let response = ui.allocate_rect(chip, Sense::click());
        painter.rect_filled(
            chip,
            CornerRadius::same(5),
            Color32::from_rgba_unmultiplied(8, 9, 12, if response.hovered() { 225 } else { 185 }),
        );
        painter.text(
            pos2(chip.left() + 6.0, chip.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(8.0),
            if response.hovered() { TEXT } else { accent },
        );
        if response.clicked() {
            self.link.send(MiniCommand::CycleViz);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }

    fn paint_spectrum(&self, painter: &egui::Painter, rect: Rect, accent: Color32) {
        let gap = 2.4;
        let bar_width =
            ((rect.width() - gap * (MINI_BAND_COUNT - 1) as f32) / MINI_BAND_COUNT as f32).max(1.0);
        let baseline = rect.bottom() - 1.0;
        let height = rect.height() - 4.0;

        for i in 0..MINI_BAND_COUNT {
            let value = self.smooth[i].clamp(0.0, 1.0);
            let peak = self.peaks[i].clamp(0.0, 1.0);
            let x = rect.left() + i as f32 * (bar_width + gap);
            let bar_height = 2.0 + value * height;
            let bar = Rect::from_min_max(
                pos2(x, baseline - bar_height),
                pos2(x + bar_width, baseline),
            );
            let color = scale(accent, 0.38 + 0.62 * value);
            painter.rect_filled(
                bar,
                CornerRadius::same((bar_width * 0.5).clamp(1.0, 3.0) as u8),
                color,
            );

            if peak > 0.03 {
                let y = baseline - 2.0 - peak * height;
                painter.line_segment(
                    [pos2(x, y), pos2(x + bar_width, y)],
                    Stroke::new(1.0_f32, scale(accent, 0.85)),
                );
            }
        }
    }

    fn paint_progress(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        now: f64,
        accent: Color32,
    ) {
        let top = content.top() + 126.0;
        let row = Rect::from_min_max(pos2(content.left(), top), pos2(content.right(), top + 18.0));
        let center_y = row.center().y;
        let duration = self.snapshot.duration_ms;
        let position = self.position_ms(now);

        painter.text(
            pos2(row.left(), center_y),
            Align2::LEFT_CENTER,
            format_time(position),
            FontId::monospace(10.0),
            MUTED,
        );
        painter.text(
            pos2(row.right(), center_y),
            Align2::RIGHT_CENTER,
            if duration > 0 {
                format_time(duration)
            } else {
                "--:--".to_owned()
            },
            FontId::monospace(10.0),
            MUTED,
        );

        let bar = Rect::from_min_max(
            pos2(row.left() + 46.0, center_y - 2.0),
            pos2(row.right() - 46.0, center_y + 2.0),
        );
        let hit = Rect::from_min_max(
            pos2(bar.left() - 4.0, row.top()),
            pos2(bar.right() + 4.0, row.bottom()),
        );
        let response = ui.allocate_rect(hit, Sense::click_and_drag());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        if duration > 0 && bar.width() > 1.0 {
            let at_pointer = |x: f32| -> u64 {
                let ratio = ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                (ratio * duration as f32).round() as u64
            };

            if response.drag_stopped() {
                let target = response
                    .interact_pointer_pos()
                    .map(|pointer| at_pointer(pointer.x))
                    .or(self.seek_preview);
                if let Some(target) = target {
                    self.commit_seek(target, now);
                }
            } else if response.drag_started() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let target = at_pointer(pointer.x);
                    self.seek_preview = Some(target);
                    self.send_seek(target, now, true);
                }
            } else if response.dragged() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let target = at_pointer(pointer.x);
                    self.seek_preview = Some(target);
                    self.send_seek(target, now, false);
                }
            } else if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    self.commit_seek(at_pointer(pointer.x), now);
                }
            }
        }

        let shown = self.position_ms(now);
        let ratio = if duration > 0 {
            (shown as f32 / duration as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fill = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * ratio, bar.max.y));

        painter.rect_filled(bar, CornerRadius::same(2), FAINT);
        if ratio > 0.0 {
            painter.rect_filled(fill, CornerRadius::same(2), scale(accent, 0.92));
        }
        if duration > 0 {
            let radius = if response.hovered() { 5.0 } else { 3.8 };
            painter.circle_filled(pos2(fill.right(), center_y), radius, accent);
        }
    }

    fn paint_controls(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        now: f64,
        accent: Color32,
    ) {
        let row = Rect::from_min_max(
            pos2(content.left(), content.top() + 152.0),
            pos2(content.right(), content.top() + 184.0),
        );
        let cy = row.center().y;

        let shuffle_rect = icon_rect(content.left(), cy, 28.0);
        let prev_rect = icon_rect(content.left() + 32.0, cy, 28.0);
        let play_rect = icon_rect(content.left() + 64.0, cy, 32.0);
        let next_rect = icon_rect(content.left() + 100.0, cy, 28.0);
        let repeat_rect = icon_rect(content.left() + 132.0, cy, 28.0);

        if icon_button(
            ui,
            shuffle_rect,
            Icon::Shuffle,
            self.snapshot.shuffle,
            accent,
            false,
        )
        .on_hover_text(if self.snapshot.shuffle {
            "aleatorio: sí"
        } else {
            "aleatorio: no"
        })
        .clicked()
        {
            self.link.send(MiniCommand::Shuffle);
        }

        if icon_button(ui, prev_rect, Icon::Previous, false, accent, false)
            .on_hover_text("anterior")
            .clicked()
        {
            self.link.send(MiniCommand::Previous);
        }

        let play_icon = if self.snapshot.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        if icon_button(ui, play_rect, play_icon, false, accent, true)
            .on_hover_text(if self.snapshot.playing {
                "pausa"
            } else {
                "reproducir"
            })
            .clicked()
        {
            self.play_local = !self.snapshot.playing;
            self.play_hold_until = now + 0.9;
            self.snapshot.playing = self.play_local;
            self.snapshot_at = now;
            self.link.send(MiniCommand::PlayPause);
        }

        if icon_button(ui, next_rect, Icon::Next, false, accent, false)
            .on_hover_text("siguiente")
            .clicked()
        {
            self.link.send(MiniCommand::Next);
        }

        let repeat_mode = self.snapshot.repeat_mode();
        let repeat_active = repeat_mode != "off";
        if icon_button(
            ui,
            repeat_rect,
            Icon::Repeat(repeat_mode == "one"),
            repeat_active,
            accent,
            false,
        )
        .on_hover_text(format!("repeat: {repeat_mode}"))
        .clicked()
        {
            self.link.send(MiniCommand::CycleRepeat);
        }

        // Volumen: icono + slider 0..100 + porcentaje.
        let volume_label_right = content.right();
        let volume_number_left = volume_label_right - 30.0;
        let slider = Rect::from_min_max(
            pos2(volume_number_left - 88.0, cy - 2.0),
            pos2(volume_number_left - 4.0, cy + 2.0),
        );
        let speaker = Rect::from_center_size(pos2(slider.left() - 16.0, cy), Vec2::splat(18.0));

        let volume_hit = Rect::from_min_max(
            pos2(slider.left() - 4.0, row.top()),
            pos2(slider.right() + 4.0, row.bottom()),
        );
        let volume_response = ui.allocate_rect(volume_hit, Sense::click_and_drag());
        if volume_response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if volume_response.drag_stopped() || volume_response.clicked() {
            let ratio = volume_response
                .interact_pointer_pos()
                .map(|pointer| ((pointer.x - slider.left()) / slider.width()).clamp(0.0, 1.0))
                .unwrap_or_else(|| clean_volume(self.volume_local));
            self.volume_local = ratio;
            self.volume_hold_until = now + 0.45;
            self.volume_sent_at = now;
            self.link.send(MiniCommand::set_volume(ratio));
        } else if volume_response.dragged() {
            if let Some(pointer) = volume_response.interact_pointer_pos() {
                let ratio = ((pointer.x - slider.left()) / slider.width()).clamp(0.0, 1.0);
                self.volume_local = ratio;
                self.volume_hold_until = now + 0.45;
                if now - self.volume_sent_at >= 0.08 {
                    self.volume_sent_at = now;
                    self.link.send(MiniCommand::set_volume(ratio));
                }
            }
        }

        draw_speaker(painter, speaker, MUTED);
        painter.rect_filled(slider, CornerRadius::same(2), FAINT);
        let volume_fill = Rect::from_min_max(
            slider.min,
            pos2(
                slider.left() + slider.width() * clean_volume(self.volume_local),
                slider.max.y,
            ),
        );
        painter.rect_filled(volume_fill, CornerRadius::same(2), scale(accent, 0.85));
        painter.circle_filled(
            pos2(
                slider.left() + slider.width() * clean_volume(self.volume_local),
                cy,
            ),
            if volume_response.hovered() { 4.5 } else { 3.5 },
            accent,
        );
        painter.text(
            pos2(volume_label_right, cy),
            Align2::RIGHT_CENTER,
            format!(
                "{}%",
                (clean_volume(self.volume_local) * 100.0).round() as u8
            ),
            FontId::monospace(9.0),
            MUTED,
        );
    }
}

impl eframe::App for MiniApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().close_requested()) && !self.quit_sent {
            self.quit_sent = true;
            self.link.send(MiniCommand::Quit);
        }

        self.drain_events(ctx);

        let now = ctx.input(|input| input.time);
        let dt = if self.last_frame > 0.0 {
            (now - self.last_frame).clamp(0.0, 0.1) as f32
        } else {
            1.0 / 30.0
        };
        self.last_frame = now;

        self.animate(now, dt);
        self.ensure_art(ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.paint(ui, now));

        ctx.request_repaint_after(std::time::Duration::from_millis(FRAME_MS));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [
            BG.r() as f32 / 255.0,
            BG.g() as f32 / 255.0,
            BG.b() as f32 / 255.0,
            1.0,
        ]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.link.shutdown();
    }
}

#[derive(Clone, Copy)]
enum Icon {
    Shuffle,
    Previous,
    Play,
    Pause,
    Next,
    Repeat(bool),
}

fn icon_rect(left: f32, center_y: f32, size: f32) -> Rect {
    Rect::from_min_size(pos2(left, center_y - size * 0.5), Vec2::splat(size))
}

#[allow(clippy::too_many_arguments)]
fn icon_button(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: Icon,
    active: bool,
    accent: Color32,
    primary: bool,
) -> Response {
    let response = ui.allocate_rect(rect, Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();

    let color = if primary {
        let background = if hovered {
            mix(accent, Color32::WHITE, 0.16)
        } else {
            accent
        };
        painter.rect_filled(rect, CornerRadius::same(7), background);
        on_accent(background)
    } else if active {
        painter.rect_filled(
            rect,
            CornerRadius::same(7),
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 28),
        );
        accent
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::same(7), PANEL_HI);
        TEXT
    } else {
        MUTED
    };

    draw_icon(painter, rect, icon, color);
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

const ICON_PLAY: &[&str] = &["█    ", "██   ", "███  ", "██   ", "█    "];
const ICON_PAUSE: &[&str] = &["█  █", "█  █", "█  █", "█  █", "█  █"];
const ICON_PREV: &[&str] = &["█  █ ", "██ █ ", "███  ", "██ █ ", "█  █ "];
const ICON_NEXT: &[&str] = &[" █  █", " █ ██", "  ███", " █ ██", " █  █"];
const ICON_SHUFFLE: &[&str] = &[
    "██   ██",
    " ██ ██ ",
    "  ███  ",
    " ██ ██ ",
    "██   ██",
];
const ICON_REPEAT: &[&str] = &[
    "███████",
    "█     █",
    "█     █",
    "█     █",
    "███████",
];
const ICON_REPEAT_ONE: &[&str] = &[
    "███████",
    "█     █",
    "█  █  █",
    "█  █  █",
    "███████",
];

/// Iconos de bloque, los mismos que la barra de transporte de la TUI.
fn draw_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let grid = match icon {
        Icon::Shuffle => ICON_SHUFFLE,
        Icon::Previous => ICON_PREV,
        Icon::Play => ICON_PLAY,
        Icon::Pause => ICON_PAUSE,
        Icon::Next => ICON_NEXT,
        Icon::Repeat(true) => ICON_REPEAT_ONE,
        Icon::Repeat(false) => ICON_REPEAT,
    };
    let rows = grid.len() as f32;
    let cols = grid.iter().map(|line| line.chars().count()).max().unwrap_or(1) as f32;
    let cell = (rect.width() / cols).min(rect.height() / rows) * 0.82;
    if cell <= 0.4 {
        return;
    }
    let width = cols * cell;
    let height = rows * cell;
    let x0 = rect.center().x - width * 0.5;
    let y0 = rect.center().y - height * 0.5;
    for (y, line) in grid.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let cell_rect = Rect::from_min_size(
                pos2(x0 + x as f32 * cell, y0 + y as f32 * cell),
                Vec2::splat((cell - 0.35).max(0.7)),
            );
            painter.rect_filled(cell_rect, CornerRadius::same(0), color);
        }
    }
}

fn draw_close(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let arm = rect.width() * 0.20;
    let stroke = Stroke::new(1.5_f32, color);
    painter.line_segment(
        [
            pos2(center.x - arm, center.y - arm),
            pos2(center.x + arm, center.y + arm),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(center.x - arm, center.y + arm),
            pos2(center.x + arm, center.y - arm),
        ],
        stroke,
    );
}

fn draw_speaker(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let size = rect.width().min(rect.height());
    let stroke = Stroke::new(1.5_f32, color);

    painter.rect_filled(
        Rect::from_center_size(
            pos2(center.x - size * 0.22, center.y),
            vec2(size * 0.16, size * 0.22),
        ),
        CornerRadius::same(1),
        color,
    );
    painter.add(Shape::convex_polygon(
        vec![
            pos2(center.x - size * 0.12, center.y - size * 0.11),
            pos2(center.x + size * 0.06, center.y - size * 0.30),
            pos2(center.x + size * 0.06, center.y + size * 0.30),
            pos2(center.x - size * 0.12, center.y + size * 0.11),
        ],
        color,
        Stroke::NONE,
    ));
    painter.line_segment(
        [
            pos2(center.x + size * 0.14, center.y - size * 0.13),
            pos2(center.x + size * 0.24, center.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            pos2(center.x + size * 0.24, center.y),
            pos2(center.x + size * 0.14, center.y + size * 0.13),
        ],
        stroke,
    );
}

fn paint_text(
    ui: &egui::Ui,
    pos: Pos2,
    text: &str,
    font_id: FontId,
    color: Color32,
    max_width: f32,
) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::text::TextFormat {
            font_id,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos, galley, color);
}

fn track_subtitle(snapshot: &MiniSnapshot) -> String {
    let mut parts = Vec::with_capacity(2);
    if !snapshot.artist.trim().is_empty() {
        parts.push(snapshot.artist.trim().to_owned());
    }
    if let Some(album) = snapshot.album.as_deref() {
        let album = album.trim();
        if !album.is_empty() {
            parts.push(album.to_owned());
        }
    }
    if parts.is_empty() {
        "sin artista".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn accent_color(rgb: [u8; 3]) -> Color32 {
    if rgb[0].max(rgb[1]).max(rgb[2]) < 40 {
        DEFAULT_ACCENT
    } else {
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }
}

/// Color legible encima de un fondo `accent`.
fn on_accent(background: Color32) -> Color32 {
    let luminance = 0.2126 * background.r() as f32
        + 0.7152 * background.g() as f32
        + 0.0722 * background.b() as f32;
    if luminance > 140.0 {
        Color32::from_rgb(12, 13, 17)
    } else {
        Color32::WHITE
    }
}

fn clean_volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn clean_band(band: f32) -> f32 {
    if band.is_finite() {
        band.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn scale(color: Color32, factor: f32) -> Color32 {
    let factor = factor.clamp(0.0, 1.0);
    Color32::from_rgb(
        (color.r() as f32 * factor) as u8,
        (color.g() as f32 * factor) as u8,
        (color.b() as f32 * factor) as u8,
    )
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}
