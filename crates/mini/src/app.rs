//! Ventana del mini reproductor.
//!
//! La UI imita el transporte de la TUI: tipografía monoespaciada, barras de
//! bloques, chips planos y botones pixel-art. El visual de la ventana grande
//! ocupa todo el fondo y la portada del disco suena a la izquierda. No usa
//! widgets de formulario salvo las zonas interactivas (`ui.allocate_rect`),
//! así el resultado es igual en X11 y Wayland.

use crate::net::{MiniLink, NetEvent};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use spote_protocol::mini::{
    format_time, mini_socket_path, MiniCommand, MiniSnapshot, MINI_BAND_COUNT,
};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const BG: Color32 = Color32::from_rgb(7, 8, 11);
const PANEL: Color32 = Color32::from_rgb(17, 18, 24);
const PANEL_HI: Color32 = Color32::from_rgb(26, 28, 36);
const TEXT: Color32 = Color32::from_rgb(237, 238, 243);
const MUTED: Color32 = Color32::from_rgb(140, 144, 157);
const FAINT: Color32 = Color32::from_rgb(52, 55, 66);
const DEFAULT_ACCENT: Color32 = Color32::from_rgb(92, 196, 170);

const ART_SIZE: f32 = 92.0;
const CORNER: u8 = 2;

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
    art_fetch: Option<Receiver<(u32, u32, Vec<u8>)>>,

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
            art_fetch: None,
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
        // Resultado de la descarga remota en curso.
        if let Some(rx) = &self.art_fetch {
            match rx.try_recv() {
                Ok((width, height, data)) => {
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &data,
                    );
                    self.texture = Some(ctx.load_texture(
                        format!("spote-mini-art-url:{}", self.snapshot.track_key),
                        color_image,
                        TextureOptions::LINEAR,
                    ));
                    self.art_fetch = None;
                }
                Err(TryRecvError::Disconnected) => self.art_fetch = None,
                Err(TryRecvError::Empty) => {}
            }
        }

        let wanted = self
            .snapshot
            .art_path
            .clone()
            .map(|path| format!("{}|path|{path}", self.snapshot.track_key))
            .or_else(|| {
                self.snapshot
                    .art_url
                    .clone()
                    .map(|url| format!("{}|url|{url}", self.snapshot.track_key))
            });
        if wanted == self.art_source {
            return;
        }
        self.art_source = wanted.clone();
        self.texture = None;
        self.art_fetch = None;

        if let Some(path) = self.snapshot.art_path.clone() {
            // `image::open` adivina el formato por la extensión y los cachés
            // de spote usan `.img`; se lee el contenido y se detecta por magic.
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(image) = image::load_from_memory(&bytes) {
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
            return;
        }

        let Some(url) = self.snapshot.art_url.clone() else {
            return;
        };
        // Descarga fuera del hilo de egui: una portada remota lenta no debe
        // congelar la ventana ni la reproducción.
        let (tx, rx) = mpsc::channel();
        self.art_fetch = Some(rx);
        std::thread::spawn(move || {
            let fetched = reqwest::blocking::get(&url)
                .and_then(|response| response.error_for_status())
                .ok()
                .and_then(|response| response.bytes().ok());
            let Some(bytes) = fetched else { return };
            let Ok(image) = image::load_from_memory(&bytes) else {
                return;
            };
            let image = image.thumbnail(320, 320);
            let rgba = image.to_rgba8();
            let _ = tx.send((rgba.width(), rgba.height(), rgba.as_raw().to_vec()));
        });
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
        let painter = ui.painter().with_clip_rect(full);

        painter.rect_filled(full, CornerRadius::same(CORNER), BG);
        let has_viz = self.snapshot.viz_rows > 0
            && self.snapshot.viz_cols > 0
            && !self.snapshot.viz.is_empty();
        if has_viz {
            self.paint_viz(&painter, full);
        } else {
            self.paint_spectrum(&painter, full, accent);
        }

        // El visual ocupa toda la ventana. El velo solo baja el contraste lo
        // justo para que portada, texto y controles se lean encima.
        painter.rect_filled(
            full,
            CornerRadius::same(CORNER),
            Color32::from_rgba_unmultiplied(7, 8, 11, 100),
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(full.left(), full.top() + 104.0), full.max),
            CornerRadius::same(0),
            Color32::from_rgba_unmultiplied(7, 8, 11, 82),
        );
        painter.rect_stroke(
            full,
            CornerRadius::same(CORNER),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 14)),
            StrokeKind::Inside,
        );

        let content = full.shrink2(vec2(14.0, 12.0));
        self.paint_header(ui, &painter, content, now, accent);
        self.paint_art(&painter, content, accent);
        self.paint_progress(ui, &painter, content, now, accent);
        self.paint_controls(ui, &painter, content, now, accent);
        self.paint_chips(ui, &painter, content, accent);

        if !self.connected {
            let overlay =
                Rect::from_min_max(pos2(content.left(), content.top() + 38.0), content.max);
            painter.rect_filled(
                overlay,
                CornerRadius::same(2),
                Color32::from_rgba_unmultiplied(7, 8, 11, 235),
            );
            painter.text(
                overlay.center() - vec2(0.0, 8.0),
                Align2::CENTER_CENTER,
                "spote no está abierto",
                FontId::monospace(12.0),
                TEXT,
            );
            painter.text(
                overlay.center() + vec2(0.0, 10.0),
                Align2::CENTER_CENTER,
                "reintentando cada 1 s",
                FontId::monospace(9.5),
                MUTED,
            );
        }
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
            pos2(content.right() - 22.0, content.top() + 2.0),
            Vec2::splat(22.0),
        );
        let dot_center = pos2(close.left() - 13.0, content.top() + 13.0);

        let drag = ui.allocate_rect(
            Rect::from_min_max(
                content.left_top(),
                pos2(close.left() - 6.0, content.top() + 38.0),
            ),
            Sense::click_and_drag(),
        );
        if drag.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        let title_right = dot_center.x - 12.0;
        let title_width = (title_right - content.left()).max(32.0);
        let title = if self.snapshot.title.is_empty() {
            "Sin reproducción"
        } else {
            self.snapshot.title.as_str()
        };
        paint_text(
            ui,
            pos2(content.left(), content.top()),
            title,
            FontId::monospace(15.0),
            TEXT,
            title_width,
        );

        let subtitle = track_subtitle(&self.snapshot);
        paint_text(
            ui,
            pos2(content.left(), content.top() + 21.0),
            &subtitle,
            FontId::monospace(9.5),
            MUTED,
            title_width,
        );

        let dot = ui.allocate_rect(
            Rect::from_center_size(dot_center, Vec2::splat(14.0)),
            Sense::hover(),
        );
        if self.connected {
            painter.circle_filled(dot_center, 3.2, accent);
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
            painter.rect_filled(close, CornerRadius::same(2), PANEL_HI);
        }
        draw_close(
            painter,
            close.shrink(7.0),
            if close_response.hovered() {
                TEXT
            } else {
                MUTED
            },
        );
        if close_response.clicked() {
            self.request_close(ui.ctx());
        }
    }

    fn paint_art(&self, painter: &egui::Painter, content: Rect, accent: Color32) {
        let art = art_rect(content);
        painter.rect_filled(
            art,
            CornerRadius::same(2),
            Color32::from_rgba_unmultiplied(10, 11, 15, 200),
        );
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                art,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.text(
                art.center() - vec2(0.0, 6.0),
                Align2::CENTER_CENTER,
                "♪",
                FontId::monospace(26.0),
                scale(accent, 0.8),
            );
            painter.text(
                art.center() + vec2(0.0, 16.0),
                Align2::CENTER_CENTER,
                "sin portada",
                FontId::monospace(8.0),
                MUTED,
            );
        }
        painter.rect_stroke(
            art,
            CornerRadius::same(2),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
            StrokeKind::Inside,
        );
        // Remate de acento, como el borde inferior del transporte de la TUI.
        painter.line_segment(
            [
                pos2(art.left(), art.bottom()),
                pos2(art.right(), art.bottom()),
            ],
            Stroke::new(2.0_f32, scale(accent, 0.85)),
        );
    }

    /// Pinta el frame del visual de la TUI sobre toda la ventana.
    ///
    /// La rejilla llega como caracteres de bloque; se convierten a rectángulos
    /// para que el fondo sea pixel-art nítido y barato de dibujar (miles de
    /// glifos por frame harían temblar la ventana).
    fn paint_viz(&self, painter: &egui::Painter, rect: Rect) {
        let cols = self.snapshot.viz_cols.max(1) as f32;
        let rows = self.snapshot.viz_rows.max(1) as f32;
        let cell_w = rect.width() / cols;
        let cell_h = rect.height() / rows;
        let mut mesh = egui::Mesh::default();
        let lift = |c: u8| -> u8 { (f32::from(c) * 1.22 + 12.0).min(255.0) as u8 };
        let mut glyphs: Vec<(Pos2, char, Color32)> = Vec::new();
        for cell in &self.snapshot.viz {
            if cell.alpha < 6 || cell.ch == ' ' {
                continue;
            }
            let color = Color32::from_rgba_unmultiplied(
                lift(cell.fg[0]),
                lift(cell.fg[1]),
                lift(cell.fg[2]),
                cell.alpha,
            );
            let Some((fx, fy, fw, fh, scale_alpha)) = viz_ink(cell.ch) else {
                glyphs.push((
                    pos2(
                        rect.left() + (cell.x as f32 + 0.5) * cell_w,
                        rect.top() + (cell.y as f32 + 0.5) * cell_h,
                    ),
                    cell.ch,
                    color,
                ));
                continue;
            };
            let x0 = rect.left() + cell.x as f32 * cell_w + fx * cell_w;
            let y0 = rect.top() + cell.y as f32 * cell_h + fy * cell_h;
            let x1 = (x0 + fw * cell_w).min(rect.right());
            let y1 = (y0 + fh * cell_h).min(rect.bottom());
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let alpha = (f32::from(color.a()) * scale_alpha)
                .round()
                .clamp(0.0, 255.0) as u8;
            mesh.add_colored_rect(
                Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)),
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
            );
        }
        painter.add(Shape::mesh(mesh));
        // Los caracteres que no son bloques (dígitos, líneas finas, braille)
        // conservan su glifo: son pocos y así el preset no pierde detalle.
        let font = FontId::monospace((cell_h * 0.98).min(cell_w * 1.2).max(4.0));
        for (pos, ch, color) in glyphs {
            painter.text(pos, Align2::CENTER_CENTER, ch, font.clone(), color);
        }
    }

    fn paint_chips(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        accent: Color32,
    ) {
        let column = right_column(content);
        let row = Rect::from_min_size(
            pos2(column.left(), content.top() + 112.0),
            vec2(column.width(), 16.0),
        );
        let label = if self.snapshot.viz_preset.is_empty() {
            "viz".to_string()
        } else {
            format!("viz · {}", self.snapshot.viz_preset)
        };
        let width = (label.chars().count() as f32 * 6.0 + 14.0).max(34.0);
        let chip = Rect::from_min_size(row.min, vec2(width, 16.0));
        let response = ui.allocate_rect(chip, Sense::click());
        painter.rect_filled(
            chip,
            CornerRadius::same(2),
            if response.hovered() {
                PANEL_HI
            } else {
                Color32::from_rgba_unmultiplied(22, 24, 31, 200)
            },
        );
        painter.text(
            pos2(chip.center().x, chip.center().y),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(8.5),
            if response.hovered() { TEXT } else { accent },
        );
        if response.clicked() {
            self.link.send(MiniCommand::CycleViz);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        painter.text(
            pos2(chip.right() + 10.0, chip.center().y),
            Align2::LEFT_CENTER,
            "clic: cambia el visual",
            FontId::monospace(8.0),
            FAINT,
        );
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
            painter.rect_filled(bar, CornerRadius::same(0), color);

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
        let column = right_column(content);
        let row = Rect::from_min_max(
            pos2(column.left(), content.top() + 38.0),
            pos2(column.right(), content.top() + 58.0),
        );
        let center_y = row.center().y;
        let duration = self.snapshot.duration_ms;
        let position = self.position_ms(now);

        painter.text(
            pos2(row.left(), center_y),
            Align2::LEFT_CENTER,
            format_time(position),
            FontId::monospace(9.0),
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
            FontId::monospace(9.0),
            FAINT,
        );

        // Barra de bloques como la del transporte: ━ para lo reproducido,
        // ─ para lo pendiente y ● / ○ como cabeza.
        let bar_font = FontId::monospace(12.0);
        let cell = ui.fonts(|f| f.glyph_width(&bar_font, '━')).max(4.0);
        let left = row.left() + 42.0;
        let right = row.right() - 42.0;
        let count = (((right - left) / cell).floor() as usize).max(8);
        let width = count as f32 * cell;
        let hit = Rect::from_min_max(pos2(left, row.top()), pos2(left + width, row.bottom()));
        let response = ui.allocate_rect(hit, Sense::click_and_drag());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }

        if duration > 0 && width > 1.0 {
            let at_pointer = |x: f32| -> u64 {
                let ratio = ((x - left) / width).clamp(0.0, 1.0);
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
        let filled = ((ratio * count as f32).round() as usize).min(count);
        if filled > 0 {
            painter.text(
                pos2(left, center_y),
                Align2::LEFT_CENTER,
                "━".repeat(filled),
                bar_font.clone(),
                scale(accent, 0.95),
            );
        }
        if filled < count {
            painter.text(
                pos2(left + filled as f32 * cell, center_y),
                Align2::LEFT_CENTER,
                "─".repeat(count - filled),
                bar_font,
                Color32::from_rgba_unmultiplied(150, 154, 168, 90),
            );
        }
        let head = left + (ratio * count as f32).floor() * cell + cell * 0.5;
        painter.text(
            pos2(head, center_y),
            Align2::CENTER_CENTER,
            if self.snapshot.playing { "●" } else { "○" },
            FontId::monospace(9.0),
            mix(TEXT, accent, 0.5),
        );
    }

    fn paint_controls(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        content: Rect,
        now: f64,
        accent: Color32,
    ) {
        let column = right_column(content);
        let row = Rect::from_min_max(
            pos2(column.left(), content.top() + 64.0),
            pos2(column.right(), content.top() + 104.0),
        );
        let cy = row.center().y;
        let pulse = self.pulse();

        let gap = 4.0;
        // Proporciones del transporte de la TUI: 9 celdas para shuffle, play y
        // bucle; 7 para anterior/siguiente, con la altura de sus tres filas.
        let sizes = [44.0, 34.0, 44.0, 34.0, 44.0];
        let total = sizes.iter().sum::<f32>() + gap * (sizes.len() - 1) as f32;
        let volume_w = 144.0;
        let free = (column.width() - volume_w - 14.0 - total).max(0.0);
        let mut x = column.left() + free * 0.5;
        let mut next_button = |width: f32| -> Rect {
            let rect = Rect::from_min_size(pos2(x, cy - 12.0), vec2(width, 24.0));
            x += width + gap;
            rect
        };
        let shuffle_rect = next_button(sizes[0]);
        let prev_rect = next_button(sizes[1]);
        let play_rect = next_button(sizes[2]);
        let next_rect = next_button(sizes[3]);
        let repeat_rect = next_button(sizes[4]);

        if icon_button(
            ui,
            shuffle_rect,
            Icon::Shuffle,
            self.snapshot.shuffle,
            accent,
            false,
            pulse,
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

        if icon_button(ui, prev_rect, Icon::Previous, false, accent, false, pulse)
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
        if icon_button(ui, play_rect, play_icon, false, accent, true, pulse)
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

        if icon_button(ui, next_rect, Icon::Next, false, accent, false, pulse)
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
            pulse,
        )
        .on_hover_text(format!("repeat: {repeat_mode}"))
        .clicked()
        {
            self.link.send(MiniCommand::CycleRepeat);
        }

        // Volumen con el mismo lenguaje que la TUI: ♪ + bloques + porcentaje.
        let right = column.right();
        let percent_w = 30.0;
        let slider_w = 88.0;
        let slider_left = right - percent_w - slider_w - 6.0;
        let slider = Rect::from_min_max(
            pos2(slider_left, cy - 3.0),
            pos2(slider_left + slider_w, cy + 3.0),
        );
        let speaker = pos2(slider.left() - 13.0, cy);

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

        let muted = clean_volume(self.volume_local) <= 0.001;
        painter.text(
            speaker,
            Align2::CENTER_CENTER,
            if muted { "×" } else { "♪" },
            FontId::monospace(11.0),
            if muted {
                Color32::from_rgb(152, 96, 96)
            } else {
                MUTED
            },
        );

        let vol_font = FontId::monospace(9.0);
        let vol_cell = ui.fonts(|f| f.glyph_width(&vol_font, '█')).max(3.0);
        let vol_count = ((slider.width() / vol_cell).floor() as usize).max(8);
        let filled =
            ((clean_volume(self.volume_local) * vol_count as f32).round() as usize).min(vol_count);
        let knob = filled.min(vol_count.saturating_sub(1));
        for i in 0..vol_count {
            let (ch, color) = if i <= knob && filled > 0 {
                (if i == knob { "◉" } else { "█" }, accent)
            } else {
                ("░", Color32::from_rgba_unmultiplied(90, 94, 108, 130))
            };
            painter.text(
                pos2(slider_left + i as f32 * vol_cell, cy),
                Align2::LEFT_CENTER,
                ch,
                vol_font.clone(),
                color,
            );
        }

        painter.text(
            pos2(right, cy),
            Align2::RIGHT_CENTER,
            format!("{:>3.0}%", clean_volume(self.volume_local) * 100.0),
            FontId::monospace(9.0),
            MUTED,
        );
    }

    fn pulse(&self) -> f32 {
        let low: f32 = self.smooth.iter().take(6).copied().sum::<f32>() / 6.0;
        (low * 1.4).clamp(0.0, 1.0)
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

fn art_rect(content: Rect) -> Rect {
    Rect::from_min_size(
        pos2(content.left(), content.top() + 40.0),
        Vec2::splat(ART_SIZE),
    )
}

fn right_column(content: Rect) -> Rect {
    Rect::from_min_max(
        pos2(content.left() + ART_SIZE + 18.0, content.top()),
        pos2(content.right(), content.bottom()),
    )
}

/// Tinta de cada carácter del visual dentro de su celda: posición y tamaño
/// relativos. Así `█` llena la celda y `▁`/`▔` son una franja fina. Los
/// caracteres que no son bloques se dibujan con su glifo.
fn viz_ink(ch: char) -> Option<(f32, f32, f32, f32, f32)> {
    Some(match ch {
        '█' => (0.0, 0.0, 1.0, 1.0, 1.0),
        '▇' => (0.0, 0.125, 1.0, 0.875, 1.0),
        '▆' => (0.0, 0.25, 1.0, 0.75, 1.0),
        '▅' => (0.0, 0.375, 1.0, 0.625, 1.0),
        '▄' => (0.0, 0.5, 1.0, 0.5, 1.0),
        '▃' => (0.0, 0.625, 1.0, 0.375, 1.0),
        '▂' => (0.0, 0.75, 1.0, 0.25, 1.0),
        '▁' => (0.0, 0.875, 1.0, 0.125, 1.0),
        '▔' => (0.0, 0.0, 1.0, 0.14, 1.0),
        '━' => (0.0, 0.38, 1.0, 0.24, 1.0),
        '─' => (0.0, 0.46, 1.0, 0.1, 1.0),
        '│' | '║' => (0.44, 0.0, 0.12, 1.0, 1.0),
        '═' => (0.0, 0.36, 1.0, 0.28, 1.0),
        '░' => (0.0, 0.0, 1.0, 1.0, 0.34),
        '▒' => (0.0, 0.0, 1.0, 1.0, 0.58),
        '▓' => (0.0, 0.0, 1.0, 1.0, 0.8),
        '·' | '.' => (0.37, 0.37, 0.26, 0.26, 1.0),
        '•' | '●' | '◉' | '○' => (0.26, 0.26, 0.48, 0.48, 1.0),
        '⣿' => (0.0, 0.0, 1.0, 1.0, 1.0),
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
fn icon_button(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: Icon,
    active: bool,
    accent: Color32,
    primary: bool,
    pulse: f32,
) -> Response {
    let response = ui.allocate_rect(rect, Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    let rounding = CornerRadius::same(2);

    if primary {
        // Halo tenue y fondo acentuado que late con los graves, como el
        // bloom del play en la TUI.
        painter.rect_filled(
            rect.expand(3.0 + pulse * 2.0),
            CornerRadius::same(3),
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                (12.0 + pulse * 26.0) as u8,
            ),
        );
    }
    let background = if primary {
        mix(PANEL, accent, 0.32 + pulse * 0.24)
    } else if active {
        mix(PANEL, accent, 0.26)
    } else if hovered {
        PANEL_HI
    } else {
        Color32::from_rgba_unmultiplied(17, 18, 24, 205)
    };
    painter.rect_filled(rect, rounding, background);
    let color = if primary || active {
        TEXT
    } else if hovered {
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
const ICON_SHUFFLE: &[&str] = &["██   ██", " ██ ██ ", "  ███  ", " ██ ██ ", "██   ██"];
const ICON_REPEAT: &[&str] = &["███████", "█     █", "█     █", "█     █", "███████"];
const ICON_REPEAT_ONE: &[&str] = &["███████", "█     █", "█  █  █", "█  █  █", "███████"];

fn draw_close(painter: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.4_f32, color);
    painter.line_segment([rect.left_top(), rect.right_bottom()], stroke);
    painter.line_segment([rect.left_bottom(), rect.right_top()], stroke);
}

/// Iconos de bloque, los mismos que la barra de transporte de la TUI.
///
/// La barra dibuja el arte de cinco filas muestreado a tres (filas 0, 1 y 3)
/// cuando el transporte tiene cuatro filas, que es el tamaño por defecto. El
/// mini usa ese mismo muestreo para que los botones enseñen exactamente la
/// misma imagen, solo que a mayor resolución.
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
    let h = grid.len();
    let keep = 3.min(h);
    let rows: Vec<&str> = (0..keep).map(|i| grid[(i * h / keep).min(h - 1)]).collect();
    let row_count = rows.len() as f32;
    let cols = grid
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1) as f32;
    let cell = (rect.height() * 0.86 / row_count).min(rect.width() * 0.90 / cols);
    if cell <= 0.4 {
        return;
    }
    let width = cols * cell;
    let height = row_count * cell;
    let x0 = rect.center().x - width * 0.5;
    let y0 = rect.center().y - height * 0.5;
    for (y, line) in rows.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let cell_rect = Rect::from_min_size(
                pos2(x0 + x as f32 * cell, y0 + y as f32 * cell),
                Vec2::splat((cell - 0.2).max(0.5)),
            );
            painter.rect_filled(cell_rect, CornerRadius::same(0), color);
        }
    }
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
