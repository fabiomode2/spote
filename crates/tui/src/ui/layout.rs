//! Motor de layout: paneles declarativos, docks izquierda/derecha y centrado
//! vertical real. Todo sale de `LayoutCfg`, así que el editor de diseño puede
//! cambiar la geometría en caliente sin tocar el dibujo.

use ratatui::layout::Rect;

use spote_core::config::LayoutCfg;

use crate::app::Fullscreen;

#[derive(Debug, Clone, Copy, Default)]
pub struct Layout {
    pub full: Rect,
    pub top: Rect,
    pub main: Rect,
    pub nav: Rect,
    pub center: Rect,
    pub side: Rect,
    pub queue: Rect,
    pub info: Rect,
    pub viz: Rect,
    pub transport: Rect,
    pub hint: Rect,
    pub art: Rect,
    pub content: Rect,
    /// Asas de 1 celda para redimensionar con el ratón.
    pub nav_split: Option<Rect>,
    pub side_split: Option<Rect>,
    /// Altura del bloque de paneles (la zona que se centra en vertical).
    pub block: Rect,
}

impl Layout {
    pub fn compute(
        area: Rect,
        cfg: &LayoutCfg,
        show_viz: bool,
        fullscreen: Fullscreen,
        art_ratio: u16,
    ) -> Self {
        let mut out = Self {
            full: area,
            ..Default::default()
        };
        if area.width < 20 || area.height < 8 {
            out.top = Rect::new(area.x, area.y, area.width, 1);
            out.main = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
            out.center = out.main;
            out.block = out.main;
            out.hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            return out;
        }

        let top_h = u16::from(cfg.top);
        let hint_h = u16::from(cfg.hint);
        let transport_h = if cfg.transport {
            cfg.transport_height.clamp(2, 4)
        } else {
            0
        };
        let viz_h = if show_viz && cfg.viz && fullscreen == Fullscreen::None {
            cfg.viz_height.clamp(2, 16)
        } else {
            0
        };
        out.top = Rect::new(area.x, area.y, area.width, top_h);
        out.hint = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(hint_h),
            area.width,
            hint_h,
        );
        out.transport = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(hint_h + transport_h),
            area.width,
            transport_h,
        );
        out.viz = Rect::new(
            area.x,
            out.transport.y.saturating_sub(viz_h),
            area.width,
            viz_h,
        );
        let main_h = out
            .viz
            .y
            .saturating_sub(area.y + top_h)
            .max(1);
        let main = Rect::new(area.x, area.y + top_h, area.width, main_h);
        out.main = main;

        // Bloque vertical: si `vcenter`, se ajusta y se centra. El porcentaje
        // permite que el usuario decida cuánto ocupa (antes no había control).
        let block_h = if cfg.vcenter {
            ((main.height as u32 * cfg.vcenter_pct.clamp(30, 100) as u32 / 100) as u16)
                .max(10)
                .min(main.height)
        } else {
            main.height
        };
        let block_y = main.y + main.height.saturating_sub(block_h) / 2;
        let block = Rect::new(main.x, block_y, main.width, block_h);
        out.block = block;

        let nav_wanted = cfg.nav && area.width >= 62;
        let side_wanted = (cfg.queue || cfg.info) && area.width >= 96;
        let mut nav_w = if nav_wanted {
            cfg.nav_width.clamp(12, 30).min(area.width / 3)
        } else {
            0
        };
        let mut side_w = if side_wanted {
            cfg.side_width.clamp(24, 60).min(area.width / 2)
        } else {
            0
        };
        // El centro manda: si no caben los laterales, se colapsan por orden
        // de importancia (primero el lateral, luego la nav).
        let min_center = 42u16;
        if nav_w + side_w + min_center > area.width {
            side_w = 0;
        }
        if nav_w + side_w + min_center > area.width {
            nav_w = 0;
        }
        let nav_left = cfg.nav_left();
        let side_left = cfg.side_left();
        let same_side = nav_w > 0 && side_w > 0 && nav_left == side_left;
        // Si comparten lado, comparten columna: si no, se solaparían.
        let dock_w = if same_side { nav_w.max(side_w) } else { 0 };
        let left_w = if same_side && nav_left {
            dock_w
        } else if same_side {
            0
        } else {
            (if nav_left { nav_w } else { 0 }) + (if side_left { side_w } else { 0 })
        };
        let right_w = if same_side && !nav_left {
            dock_w
        } else if same_side {
            0
        } else {
            (if !nav_left { nav_w } else { 0 }) + (if !side_left { side_w } else { 0 })
        };
        out.nav = if nav_w > 0 {
            let width = if same_side { dock_w } else { nav_w };
            Rect::new(
                main.x + if nav_left { 0 } else { main.width - width },
                block.y,
                width,
                block.height,
            )
        } else {
            Rect::default()
        };
        out.side = if side_w > 0 {
            let width = if same_side { dock_w } else { side_w };
            Rect::new(
                main.x + if side_left { 0 } else { main.width - width },
                block.y,
                width,
                block.height,
            )
        } else {
            Rect::default()
        };
        // Si nav y lateral comparten lado, se apilan en vertical.
        if same_side {
            let nav_h = (block.height as f32 * 0.42) as u16;
            out.nav.height = nav_h.max(6).min(block.height);
            out.side.y = block.y + out.nav.height;
            out.side.height = block.height.saturating_sub(out.nav.height);
        }
        out.center = Rect::new(
            main.x + left_w,
            block.y,
            main.width.saturating_sub(left_w + right_w),
            block.height,
        );

        if side_w > 0 {
            let side = out.side;
            let queue_visible = cfg.queue;
            let info_visible = cfg.info;
            if queue_visible && info_visible {
                let queue_h = (side.height as f32 * (cfg.queue_share.clamp(20, 80) as f32 / 100.0))
                    as u16;
                out.queue = Rect::new(side.x, side.y, side.width, queue_h);
                out.info = Rect::new(
                    side.x,
                    side.y + queue_h,
                    side.width,
                    side.height.saturating_sub(queue_h),
                );
            } else if queue_visible {
                out.queue = side;
            } else if info_visible {
                out.info = side;
            }
        }

        // Asas de arrastre en el borde interior de cada dock.
        out.nav_split = (nav_w > 0).then(|| {
            Rect::new(
                if nav_left { out.nav.x + nav_w - 1 } else { out.nav.x },
                out.nav.y,
                1,
                out.nav.height,
            )
        });
        out.side_split = (side_w > 0).then(|| {
            Rect::new(
                if side_left { out.side.x + side_w - 1 } else { out.side.x },
                out.side.y,
                1,
                out.side.height,
            )
        });

        // Portada y texto. La celda es ~1:2: para que la portada sea cuadrada
        // en píxeles, ancho = alto * 2. Si el ancho limita, se baja el alto:
        // antes se estiraba y la portada salía deformada.
        let inner_h = out.center.height.saturating_sub(2);
        let side_pref = cfg.art_side.as_str();
        if art_ratio == 0 {
            out.content = out.center;
            return out;
        }
        if out.center.width >= 82 && inner_h >= 12 {
            let max_art_w = (out.center.width.saturating_sub(44))
                .min((out.center.width as u32 * cfg.art_size.clamp(40, 100) as u32 / 100) as u16);
            let mut art_w = (inner_h * 2).min(max_art_w.max(24));
            let mut art_h = (art_w / 2).min(inner_h);
            art_w = (art_h * 2).min(out.center.width.saturating_sub(20));
            art_h = (art_w / 2).max(2);
            let art_left = side_pref == "left";
            let art_x = if art_left {
                out.center.x + 1
            } else {
                out.center.x + out.center.width.saturating_sub(art_w + 1)
            };
            out.art = Rect::new(art_x, out.center.y + 1, art_w, art_h);
            let text_w = out.center.width.saturating_sub(art_w + 3);
            out.content = if art_left {
                Rect::new(out.center.x + art_w + 2, out.center.y, text_w, out.center.height)
            } else {
                Rect::new(out.center.x + 1, out.center.y, text_w, out.center.height)
            };
        } else if out.center.width >= 46 && out.center.height >= 14 {
            // Estrecho: miniatura arriba y el texto ocupa todo el alto. Se
            // mantiene la proporción aunque el terminal sea vertical.
            let art_w = (out.center.width / 3).clamp(16, 30);
            let art_h = (art_w / 2).min(out.center.height.saturating_sub(8)).max(2);
            let art_x = if side_pref == "left" {
                out.center.x + 1
            } else {
                out.center.x + out.center.width.saturating_sub(art_w + 2)
            };
            out.art = Rect::new(art_x, out.center.y + 1, art_w, art_h);
            out.content = out.center;
        } else {
            out.content = out.center;
        }

        // El ancho de la portada también respeta el ajuste en modo estrecho.
        if out.art.width > 0 {
            let max_w = (out.center.width as u32 * art_ratio as u32 / 10) as u16;
            if out.art.width > max_w {
                let w = max_w.max(8);
                out.art.width = w;
                out.art.height = (w / 2).max(2);
            }
        }
        out
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutCfg {
        LayoutCfg::default()
    }

    #[test]
    fn never_overlaps_or_leaves_center_empty() {
        for (w, h) in [(240, 70), (200, 56), (160, 48), (120, 36), (80, 24), (30, 10)] {
            let layout = Layout::compute(Rect::new(0, 0, w, h), &cfg(), true, Fullscreen::None, 6);
            assert!(layout.center.width > 0, "{w}x{h}");
            assert!(layout.center.right() <= w, "{w}x{h}");
            assert!(layout.main.bottom() <= h, "{w}x{h}");
            assert!(layout.art.right() <= w, "{w}x{h}");
            if layout.art.width > 0 {
                assert!(layout.art.height <= layout.center.height);
            }
        }
    }

    #[test]
    fn art_keeps_square_pixel_aspect() {
        let layout = Layout::compute(Rect::new(0, 0, 200, 60), &cfg(), true, Fullscreen::None, 6);
        if layout.art.width > 0 {
            let ratio = layout.art.width as f32 / layout.art.height as f32;
            assert!(ratio >= 1.5 && ratio <= 2.5, "ratio {ratio}");
        }
    }

    #[test]
    fn docks_can_swap_sides() {
        let mut c = cfg();
        c.nav_dock = "right".into();
        c.side_dock = "left".into();
        let layout = Layout::compute(Rect::new(0, 0, 220, 60), &c, true, Fullscreen::None, 6);
        assert_eq!(layout.nav.x + layout.nav.width, 220);
        assert_eq!(layout.side.x, 0);
    }

    #[test]
    fn same_side_docks_share_a_column() {
        let mut c = cfg();
        c.nav_dock = "right".into();
        c.side_dock = "right".into();
        let layout = Layout::compute(Rect::new(0, 0, 200, 60), &c, true, Fullscreen::None, 6);
        assert_eq!(layout.nav.x, layout.side.x);
        assert_eq!(layout.nav.width, layout.side.width);
        assert_eq!(layout.nav.x, layout.center.x + layout.center.width);
        assert!(layout.nav.y + layout.nav.height <= layout.side.y);
    }

    #[test]
    fn hiding_panels_frees_space() {
        let mut c = cfg();
        c.nav = false;
        c.queue = false;
        c.info = false;
        c.viz = false;
        let layout = Layout::compute(Rect::new(0, 0, 140, 40), &c, true, Fullscreen::None, 6);
        assert_eq!(layout.nav.width, 0);
        assert_eq!(layout.side.width, 0);
        assert_eq!(layout.viz.height, 0);
        assert_eq!(layout.center.width, 140);
    }

    #[test]
    fn zero_art_ratio_leaves_full_center_for_lists() {
        let layout = Layout::compute(Rect::new(0, 0, 160, 50), &cfg(), true, Fullscreen::None, 0);
        assert_eq!(layout.art.width, 0);
        assert_eq!(layout.content, layout.center);
    }

    #[test]
    fn fullscreen_removes_viz_strip() {
        let layout = Layout::compute(
            Rect::new(0, 0, 160, 50),
            &cfg(),
            true,
            Fullscreen::Viz,
            6,
        );
        assert_eq!(layout.viz.height, 0);
    }
}
