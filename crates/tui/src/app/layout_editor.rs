//! Editor de diseño en caliente (`E`): visibilidad, tamaño y dock de paneles.
//! Es la fuente de verdad de `[ui.layout]`; no hay geometría duplicada.

use super::*;
use spote_core::config::LayoutCfg;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayoutRow {
    Top,
    Nav,
    Queue,
    Info,
    Viz,
    Transport,
    Hint,
    Vcenter,
    VcenterPct,
    NavWidth,
    SideWidth,
    VizHeight,
    TransportHeight,
    NavDock,
    SideDock,
    QueueShare,
    ArtSize,
    ArtSide,
    Reset,
    Save,
}

const ROWS: [LayoutRow; 20] = [
    LayoutRow::Top,
    LayoutRow::Nav,
    LayoutRow::Queue,
    LayoutRow::Info,
    LayoutRow::Viz,
    LayoutRow::Transport,
    LayoutRow::Hint,
    LayoutRow::Vcenter,
    LayoutRow::VcenterPct,
    LayoutRow::NavWidth,
    LayoutRow::SideWidth,
    LayoutRow::VizHeight,
    LayoutRow::TransportHeight,
    LayoutRow::NavDock,
    LayoutRow::SideDock,
    LayoutRow::QueueShare,
    LayoutRow::ArtSize,
    LayoutRow::ArtSide,
    LayoutRow::Reset,
    LayoutRow::Save,
];

impl LayoutRow {
    fn label(self) -> &'static str {
        match self {
            Self::Top => "barra superior",
            Self::Nav => "navegación",
            Self::Queue => "cola",
            Self::Info => "información / amigos",
            Self::Viz => "franja de efectos",
            Self::Transport => "transporte",
            Self::Hint => "leyenda inferior",
            Self::Vcenter => "centrar paneles",
            Self::VcenterPct => "alto del bloque",
            Self::NavWidth => "ancho navegación",
            Self::SideWidth => "ancho lateral",
            Self::VizHeight => "alto efectos",
            Self::TransportHeight => "alto transporte",
            Self::NavDock => "lado navegación",
            Self::SideDock => "lado lateral",
            Self::QueueShare => "cola vs info",
            Self::ArtSize => "tamaño portada",
            Self::ArtSide => "lado portada",
            Self::Reset => "restaurar diseño",
            Self::Save => "guardar",
        }
    }
}

impl App {
    pub fn layout_rows(&self) -> Vec<(&'static str, String)> {
        ROWS.iter()
            .map(|row| (row.label(), self.layout_value(*row)))
            .collect()
    }

    fn layout_value(&self, row: LayoutRow) -> String {
        let l = &self.cfg.ui.layout;
        match row {
            LayoutRow::Top => onoff(l.top),
            LayoutRow::Nav => onoff(l.nav),
            LayoutRow::Queue => onoff(l.queue),
            LayoutRow::Info => onoff(l.info),
            LayoutRow::Viz => onoff(l.viz),
            LayoutRow::Transport => onoff(l.transport),
            LayoutRow::Hint => onoff(l.hint),
            LayoutRow::Vcenter => onoff(l.vcenter),
            LayoutRow::VcenterPct => format!("{}%", l.vcenter_pct),
            LayoutRow::NavWidth => format!("{} col", l.nav_width),
            LayoutRow::SideWidth => format!("{} col", l.side_width),
            LayoutRow::VizHeight => format!("{} filas", l.viz_height),
            LayoutRow::TransportHeight => format!("{} filas", l.transport_height),
            LayoutRow::NavDock => if l.nav_left() { "izquierda" } else { "derecha" }.into(),
            LayoutRow::SideDock => if l.side_left() { "izquierda" } else { "derecha" }.into(),
            LayoutRow::QueueShare => format!("{}%", l.queue_share),
            LayoutRow::ArtSize => format!("{}%", l.art_size),
            LayoutRow::ArtSide => l.art_side.clone(),
            LayoutRow::Reset => "enter".into(),
            LayoutRow::Save => "enter".into(),
        }
    }

    pub(super) fn handle_layout_key(&mut self, key: KeyEvent) {
        let len = ROWS.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('E') => self.overlay = Overlay::None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.layout_selected = self.layout_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.layout_selected = (self.layout_selected + 1).min(len - 1);
            }
            KeyCode::Left => self.adjust_layout(-1),
            KeyCode::Right => self.adjust_layout(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.toggle_layout(),
            KeyCode::Char('r') => self.reset_layout(),
            _ => {}
        }
    }

    fn toggle_layout(&mut self) {
        let row = ROWS[self.layout_selected.min(ROWS.len() - 1)];
        let l = &mut self.cfg.ui.layout;
        match row {
            LayoutRow::Top => l.top = !l.top,
            LayoutRow::Nav => l.nav = !l.nav,
            LayoutRow::Queue => l.queue = !l.queue,
            LayoutRow::Info => l.info = !l.info,
            LayoutRow::Viz => l.viz = !l.viz,
            LayoutRow::Transport => l.transport = !l.transport,
            LayoutRow::Hint => l.hint = !l.hint,
            LayoutRow::Vcenter => {
                l.vcenter = !l.vcenter;
                self.vcenter = l.vcenter;
            }
            LayoutRow::Reset => {
                *l = LayoutCfg::default();
                self.vcenter = l.vcenter;
            }
            LayoutRow::Save => {
                match self.cfg.save() {
                    Ok(()) => self.toast("diseño guardado", ToastKind::Ok),
                    Err(err) => self.toast(format!("no pude guardar: {err}"), ToastKind::Error),
                }
                self.overlay = Overlay::None;
                return;
            }
            _ => self.adjust_layout(1),
        }
        self.cfg.save().ok();
    }

    fn adjust_layout(&mut self, delta: i32) {
        let row = ROWS[self.layout_selected.min(ROWS.len() - 1)];
        let l = &mut self.cfg.ui.layout;
        match row {
            LayoutRow::NavWidth => l.nav_width = (l.nav_width as i32 + delta * 2).clamp(12, 30) as u16,
            LayoutRow::SideWidth => {
                l.side_width = (l.side_width as i32 + delta * 2).clamp(24, 60) as u16
            }
            LayoutRow::VcenterPct => {
                l.vcenter_pct = (l.vcenter_pct as i32 + delta * 4).clamp(30, 100) as u8
            }
            LayoutRow::VizHeight => {
                l.viz_height = (l.viz_height as i32 + delta).clamp(2, 16) as u16
            }
            LayoutRow::TransportHeight => {
                l.transport_height = (l.transport_height as i32 + delta).clamp(2, 4) as u16
            }
            LayoutRow::QueueShare => {
                l.queue_share = (l.queue_share as i32 + delta * 5).clamp(20, 80) as u8
            }
            LayoutRow::ArtSize => l.art_size = (l.art_size as i32 + delta * 5).clamp(40, 100) as u8,
            LayoutRow::NavDock => l.nav_dock = if l.nav_left() { "right" } else { "left" }.into(),
            LayoutRow::SideDock => {
                l.side_dock = if l.side_left() { "right" } else { "left" }.into()
            }
            LayoutRow::ArtSide => {
                l.art_side = match l.art_side.as_str() {
                    "auto" => "left",
                    "left" => "right",
                    _ => "auto",
                }
                .into()
            }
            _ => {
                self.toggle_layout();
                return;
            }
        }
        self.vcenter = self.cfg.ui.layout.vcenter;
        self.cfg.save().ok();
        self.needs_redraw = true;
    }

    fn reset_layout(&mut self) {
        self.cfg.ui.layout = LayoutCfg::default();
        self.vcenter = self.cfg.ui.layout.vcenter;
        self.cfg.save().ok();
        self.needs_redraw = true;
        self.toast("diseño restaurado", ToastKind::Ok);
    }
}

fn onoff(value: bool) -> String {
    if value { "sí".into() } else { "no".into() }
}
