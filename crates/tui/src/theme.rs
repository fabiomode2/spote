//! Animación: muelles, easings y transición de paleta entre canciones.

use std::time::{Duration, Instant};

use spote_core::artwork::Palette;

/// Muelle crítico-amortiguado. Se actualiza con dt real, así que el feel no
/// depende de los fps.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    pub velocity: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            velocity: 0.0,
            stiffness: 180.0,
            damping: 22.0,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn update(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.05);
        if self.settled() {
            self.value = self.target;
            self.velocity = 0.0;
            return;
        }
        let acceleration =
            (self.target - self.value) * self.stiffness - self.velocity * self.damping;
        self.velocity += acceleration * dt;
        self.value += self.velocity * dt;
    }

    /// Quieto en el destino (dentro del umbral visible): la UI puede dejar de
    /// repintarse por este muelle.
    pub fn settled(&self) -> bool {
        (self.value - self.target).abs() < 0.0005 && self.velocity.abs() < 0.0005
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Reloj de un canvas de efectos. Separa el ritmo al que se regenera el
/// visualizador del ritmo del bucle de la UI: si llega un frame antes de
/// tiempo, se reutiliza el canvas y el terminal no recibe ni una celda.
#[derive(Debug, Clone)]
pub struct FxClock {
    hz: f32,
    last: Instant,
}

impl FxClock {
    pub fn new(hz: f32) -> Self {
        let hz = hz.clamp(1.0, 240.0);
        Self {
            hz,
            last: Instant::now()
                .checked_sub(Duration::from_secs_f32(1.0 / hz))
                .unwrap_or_else(Instant::now),
        }
    }

    pub fn set_hz(&mut self, hz: f32) {
        self.hz = hz.clamp(1.0, 240.0);
    }

    /// `Some(dt)` cuando toca regenerar (dt real acumulado, acotado), `None`
    /// cuando el canvas anterior sigue valiendo.
    pub fn due(&mut self, now: Instant) -> Option<f32> {
        let interval = 1.0 / self.hz;
        let elapsed = now.saturating_duration_since(self.last).as_secs_f32();
        // Tolerancia de 3/4 del intervalo: si el bucle y el efecto van a la
        // misma frecuencia, el jitter hacía que se saltara un frame y el
        // efecto real cayera a la mitad (30 fps con todo a 60).
        if elapsed < interval * 0.75 {
            return None;
        }
        self.last = now;
        Some(elapsed.clamp(0.0, 0.1))
    }
}

/// Degradado de paleta al cambiar de canción. La duración la manda la config;
/// durante el fade se interpola en OKLab, no en RGB.
#[derive(Debug, Clone)]
pub struct PaletteRuntime {
    from: Palette,
    to: Palette,
    current: Palette,
    progress: f32,
    duration: f32,
}

impl PaletteRuntime {
    pub fn new(palette: Palette, duration_ms: u64) -> Self {
        Self {
            from: palette,
            to: palette,
            current: palette,
            progress: 1.0,
            duration: (duration_ms.max(1) as f32) / 1000.0,
        }
    }

    pub fn current(&self) -> &Palette {
        &self.current
    }

    /// Arranca (o redirige) el fade hacia `palette`.
    pub fn go(&mut self, palette: Palette) {
        if palette == self.to && self.progress >= 1.0 {
            return;
        }
        self.from = self.current;
        self.to = palette;
        self.progress = 0.0;
    }

    pub fn update(&mut self, dt: f32) -> bool {
        if self.progress >= 1.0 {
            return false;
        }
        self.progress = (self.progress + dt / self.duration).min(1.0);
        let eased = ease_in_out_cubic(self.progress);
        self.current = self.from.lerp(&self.to, eased);
        true
    }

    /// El degradado sigue en marcha y cambia la paleta cada frame.
    pub fn animating(&self) -> bool {
        self.progress < 1.0
    }
}

/// Pulso que sube de golpe y decae: para el playhead y los bordes en beat.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pulse {
    pub value: f32,
}

impl Pulse {
    pub fn hit(&mut self, amount: f32) {
        self.value = (self.value + amount).min(1.5);
    }

    pub fn update(&mut self, dt: f32) {
        self.value *= (-dt / 0.30).exp();
        if self.value < 0.001 {
            self.value = 0.0;
        }
    }

    pub fn eased(&self) -> f32 {
        ease_out_cubic(self.value.min(1.0))
    }
}

/// Contador de fps con media exponencial (para el overlay de debug).
#[derive(Debug, Clone)]
pub struct FpsMeter {
    last: std::time::Instant,
    smoothed: f32,
}

impl FpsMeter {
    pub fn new() -> Self {
        Self {
            last: std::time::Instant::now(),
            smoothed: 60.0,
        }
    }

    pub fn tick(&mut self) -> f32 {
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32();
        self.last = now;
        if dt > 0.0001 {
            let instant = 1.0 / dt;
            self.smoothed += (instant - self.smoothed) * 0.08;
        }
        self.smoothed
    }

    pub fn value(&self) -> f32 {
        self.smoothed
    }
}

impl Default for FpsMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_reaches_target_without_oscillating_forever() {
        let mut spring = Spring::new(0.0);
        spring.set_target(10.0);
        for _ in 0..240 {
            spring.update(1.0 / 60.0);
        }
        assert!((spring.value - 10.0).abs() < 0.05, "value={}", spring.value);
    }

    #[test]
    fn easings_are_bounded() {
        for t in [0.0, 0.3, 0.5, 0.9, 1.0] {
            assert!((0.0..=1.0).contains(&ease_out_cubic(t)));
            assert!((0.0..=1.0).contains(&ease_in_out_cubic(t)));
        }
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
    }

    #[test]
    fn palette_fade_ends_at_target() {
        let a = Palette::dark();
        let b = Palette::from_accents((240, 80, 40), (40, 200, 240));
        let mut runtime = PaletteRuntime::new(a, 100);
        runtime.go(b);
        for _ in 0..20 {
            runtime.update(1.0 / 60.0);
        }
        assert_eq!(*runtime.current(), b);
        assert_eq!(*runtime.current(), b);
    }

    #[test]
    fn pulse_decays() {
        let mut pulse = Pulse::default();
        pulse.hit(1.0);
        assert!(pulse.value > 0.9);
        for _ in 0..60 {
            pulse.update(1.0 / 60.0);
        }
        assert!(pulse.value < 0.05);
    }

    #[test]
    fn fx_clock_respects_its_rate() {
        let mut clock = FxClock::new(30.0);
        let start = Instant::now();
        // El primer frame ya vence: el canvas nunca arranca vacío.
        assert!(clock.due(start).is_some());
        assert!(clock.due(start + Duration::from_millis(5)).is_none());
        assert!(clock.due(start + Duration::from_millis(34)).is_some());
        assert!(clock.due(start + Duration::from_millis(50)).is_none());
        // Un dt enorme se acota para que la animación no dé un salto.
        let dt = clock.due(start + Duration::from_secs(2)).unwrap();
        assert!(dt <= 0.1);
    }

    #[test]
    fn palette_reports_when_it_finishes_fading() {
        let a = Palette::dark();
        let b = Palette::from_accents((240, 80, 40), (40, 200, 240));
        let mut runtime = PaletteRuntime::new(a, 100);
        assert!(!runtime.animating());
        runtime.go(b);
        assert!(runtime.animating());
        for _ in 0..20 {
            runtime.update(1.0 / 60.0);
        }
        assert!(!runtime.animating());
    }

    #[test]
    fn spring_reports_when_it_settles() {
        let mut spring = Spring::new(0.0);
        spring.set_target(4.0);
        assert!(!spring.settled());
        for _ in 0..240 {
            spring.update(1.0 / 60.0);
        }
        assert!(spring.settled());
    }
}
