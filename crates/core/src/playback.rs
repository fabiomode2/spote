//! Política pura de repetición y orden aleatorio de la cola.
//!
//! Este módulo solo decide índices: no reproduce audio, no habla con la UI y no
//! conoce el catálogo. La intención es que cualquier cliente (TUI, web, móvil)
//! pueda compartir exactamente las mismas reglas.
//!
//! El único estado mutable es [`ShuffleState`], que mantiene una permutación de
//! la cola y un cursor. Salvo el orden aleatorio de esa permutación, todas las
//! funciones son deterministas.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Modo de repetición de la cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    /// Al terminar la cola, la reproducción se detiene.
    #[default]
    Off,
    /// Al terminar la cola, vuelve a empezar.
    All,
    /// Repite indefinidamente la pista actual (lo gestiona el llamador).
    One,
}

impl RepeatMode {
    /// Ciclo `Off -> All -> One -> Off`.
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Qué debe hacer el reproductor al terminar la pista actual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// Pasar al índice indicado.
    To(usize),
    /// Volver a empezar la misma pista.
    RepeatCurrent,
    /// Detener la reproducción.
    Stop,
}

/// Sentido de un salto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Next,
    Prev,
}

/// Resultado mínimo de una operación de movimiento por la cola.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// La reproducción continúa en el índice indicado.
    To(usize),
    /// No hay destino posible.
    Stop,
}

/// Estado de reproducción aleatoria.
///
/// Invariantes:
/// - `order` es una permutación de `0..len` (cada pista exactamente una vez).
/// - `pos` apunta a la pista actual dentro de `order`.
/// - `history` guarda los índices reales ya reproducidos que permiten `prev`.
#[derive(Debug, Clone)]
pub struct ShuffleState {
    order: Vec<usize>,
    pos: usize,
    enabled: bool,
    history: Vec<usize>,
}

impl ShuffleState {
    /// Crea el estado con la pista `current` al frente y el resto barajado.
    ///
    /// `current` se recorta a `len - 1` si está fuera de rango. El estado nace
    /// desactivado; [`ShuffleState::enable`] vuelve a barajar.
    pub fn new(len: usize, current: usize) -> Self {
        let current = clamp_index(current, len);
        Self {
            order: build_order(len, current),
            pos: 0,
            enabled: false,
            history: Vec::new(),
        }
    }

    /// Activa el shuffle. Si ya estaba activo no hace nada: es idempotente y
    /// conserva el orden y el historial existentes.
    pub fn enable(&mut self, len: usize, current: usize) {
        if self.enabled {
            return;
        }
        self.reset(len, current);
        self.enabled = true;
    }

    /// Desactiva el shuffle y devuelve el índice real de la pista actual.
    ///
    /// Mientras el shuffle está activo, `current` es el índice real que el
    /// reproductor tiene cargado. El método sincroniza el cursor interno con
    /// ese índice (o reconstruye el estado si la cola cambió sin avisar) y lo
    /// devuelve. Si ya estaba desactivado, devuelve `current` recortado al
    /// rango actual. En ambos casos la llamada es idempotente.
    pub fn disable(&mut self, current: usize) -> usize {
        let current = clamp_index(current, self.order.len());
        if !self.enabled {
            return current;
        }

        self.enabled = false;
        if self.order.is_empty() {
            self.pos = 0;
            self.history.clear();
            return current;
        }

        match self.order.iter().position(|&track| track == current) {
            Some(position) => self.pos = position,
            None => {
                // La cola mutó sin pasar por `on_len_changed`. Reconstruimos
                // para no dejar el cursor sobre una pista inexistente.
                self.order = build_order(self.order.len(), current);
                self.pos = 0;
            }
        }
        self.history.clear();
        current
    }

    /// Avanza a la siguiente pista de la baraja.
    ///
    /// `RepeatMode::One` se trata como `Off`: repetir la pista actual es una
    /// decisión del reproductor y queda fuera de esta política. Si se agota el
    /// ciclo:
    /// - `All` genera una baraja nueva (la pista actual queda al frente y se
    ///   salta a la siguiente para no repetirla de inmediato).
    /// - `Off` y `One` devuelven [`Advance::Stop`] sin mover el cursor.
    pub fn next(&mut self, len: usize, repeat: RepeatMode) -> Advance {
        if len == 0 {
            return Advance::Stop;
        }
        self.ensure_len(len);

        let Some(current) = self.current_index() else {
            return Advance::Stop;
        };

        if self.pos + 1 < self.order.len() {
            self.history.push(current);
            self.pos += 1;
            return Advance::To(self.order[self.pos]);
        }

        if repeat != RepeatMode::All {
            return Advance::Stop;
        }

        // Fin de ciclo: nueva baraja con la pista que acaba de sonar al frente.
        self.history.push(current);
        self.order = build_order(len, current);
        if len == 1 {
            self.pos = 0;
            Advance::To(current)
        } else {
            self.pos = 1;
            Advance::To(self.order[1])
        }
    }

    /// Retrocede a la pista anterior del historial.
    ///
    /// Devuelve `None` si no hay historial (o si la cola está vacía). Las
    /// entradas fuera de rango se descartan: una mutación de cola sin
    /// [`ShuffleState::on_len_changed`] invalida el backup.
    pub fn prev(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            self.history.clear();
            return None;
        }
        self.ensure_len(len);

        while let Some(previous) = self.history.pop() {
            if previous >= len {
                continue;
            }
            match self.order.iter().position(|&track| track == previous) {
                Some(position) => self.pos = position,
                None => {
                    // La pista existe en la cola actual pero no en la baraja
                    // guardada; rehacemos la baraja con ella al frente.
                    self.order = build_order(len, previous);
                    self.pos = 0;
                }
            }
            return Some(previous);
        }
        None
    }

    /// Reconstruye el estado para una cola de longitud `len` distinta.
    ///
    /// Conserva la pista actual al frente y baraja el resto. **Cualquier
    /// mutación de la cola (insertar, borrar o mover) invalida el historial y
    /// la baraja anteriores**; por eso este método limpia el backup y debe
    /// llamarse después de cada mutación. El flag `enabled` no cambia.
    pub fn on_len_changed(&mut self, len: usize, current: usize) {
        self.reset(len, current);
    }

    /// Vista de solo lectura del orden actual, útil para tests y diagnóstico.
    pub fn order_snapshot(&self) -> &[usize] {
        &self.order
    }

    /// Indica si el shuffle está activo.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Reconstruye la baraja y limpia el historial, sin tocar `enabled`.
    fn reset(&mut self, len: usize, current: usize) {
        self.order = build_order(len, current);
        self.pos = 0;
        self.history.clear();
    }

    /// Repara el estado si el llamador cambió la longitud sin avisar.
    fn ensure_len(&mut self, len: usize) {
        if self.order.len() != len {
            let current = self.current_index().unwrap_or(0);
            self.reset(len, current);
        }
    }

    /// Índice real de la pista apuntada por el cursor.
    fn current_index(&self) -> Option<usize> {
        self.order.get(self.pos).copied()
    }
}

/// Calcula el índice al que se movería el cursor.
///
/// - `RepeatMode::All`: hace wrap en ambos extremos.
/// - `RepeatMode::Off` y `RepeatMode::One`: se detiene en los extremos. `One`
///   se comporta como `Off` aquí porque la repetición automática la decide
///   [`auto_advance`].
/// - `current` fuera de rango se satura al último índice válido.
/// - Una cola vacía devuelve `None`.
pub fn advance_index(
    current: usize,
    len: usize,
    repeat: RepeatMode,
    direction: Direction,
) -> Option<usize> {
    if len == 0 {
        return None;
    }

    let current = current.min(len - 1);
    match direction {
        Direction::Next => {
            if current + 1 < len {
                Some(current + 1)
            } else if repeat == RepeatMode::All {
                Some(0)
            } else {
                None
            }
        }
        Direction::Prev => {
            if current > 0 {
                Some(current - 1)
            } else if repeat == RepeatMode::All {
                Some(len - 1)
            } else {
                None
            }
        }
    }
}

/// Decisión automática al terminar una pista (sin intervención del usuario).
///
/// `One` siempre pide repetir la pista actual. `All` y `Off` delegan en
/// [`advance_index`]; cuando no hay destino, se detiene.
pub fn auto_advance(current: usize, len: usize, repeat: RepeatMode) -> Advance {
    if len == 0 {
        return Advance::Stop;
    }
    if repeat == RepeatMode::One {
        return Advance::RepeatCurrent;
    }
    match advance_index(current, len, repeat, Direction::Next) {
        Some(index) => Advance::To(index),
        None => Advance::Stop,
    }
}

/// Decisión de un salto manual hacia delante.
///
/// Un salto manual con `One` se comporta como `Off`: el usuario pidió avanzar,
/// no repetir. `All` hace wrap y `Off` se detiene al final.
pub fn manual_next(current: usize, len: usize, repeat: RepeatMode) -> Advance {
    let repeat = if repeat == RepeatMode::One {
        RepeatMode::Off
    } else {
        repeat
    };
    match advance_index(current, len, repeat, Direction::Next) {
        Some(index) => Advance::To(index),
        None => Advance::Stop,
    }
}

/// Reajusta el cursor tras mover una entrada de la cola.
///
/// `from` y `to` usan la semántica de `Vec`: primero se extrae la entrada en
/// `from` y después se inserta en la posición final `to`. Si se mueve la pista
/// actual, el cursor pasa a `to`. Si se mueve otra entrada que cruza el cursor,
/// este se desplaza una posición. Devuelve `None` si algún índice es inválido
/// o si `len` es cero.
pub fn move_index(current: usize, from: usize, to: usize, len: usize) -> Option<usize> {
    if len == 0 || current >= len || from >= len || to >= len {
        return None;
    }
    if from == current {
        return Some(to);
    }
    if from == to {
        return Some(current);
    }

    let adjusted = if from < current && to >= current {
        current - 1
    } else if from > current && to <= current {
        current + 1
    } else {
        current
    };
    Some(adjusted)
}

/// Reajusta el cursor tras borrar la entrada `index` de la cola.
///
/// Si se borra una pista anterior a la actual, el cursor retrocede una
/// posición. Si se borra una posterior, no cambia. Si se borra la actual,
/// devuelve el índice que ocupa la siguiente pista (el mismo valor numérico de
/// `current`) o `None` si la actual era la última. Devuelve `None` si algún
/// índice es inválido o si `len` es cero.
pub fn remove_index(current: usize, index: usize, len: usize) -> Option<usize> {
    if len == 0 || current >= len || index >= len {
        return None;
    }
    if index == current {
        if current + 1 < len {
            Some(current)
        } else {
            None
        }
    } else if index < current {
        Some(current - 1)
    } else {
        Some(current)
    }
}

/// Recorta un índice al rango válido de una cola.
fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        index.min(len - 1)
    }
}

/// Construye una permutación con `current` al frente y el resto barajado.
fn build_order(len: usize, current: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }

    let current = current.min(len - 1);
    let mut rest: Vec<usize> = (0..len).filter(|&index| index != current).collect();
    shuffle_in_place(&mut rest, entropy_seed());

    let mut order = Vec::with_capacity(len);
    order.push(current);
    order.extend(rest);
    order
}

/// Fisher-Yates con un SplitMix64 mínimo.
///
/// No se necesita calidad criptográfica: basta con repartir índices sin un
/// sesgo apreciable para tamaños de cola reales. El sesgo del módulo es
/// despreciable porque `bound` nunca se acerca al rango de `u64`.
fn shuffle_in_place(items: &mut [usize], seed: u64) {
    let mut rng = SplitMix64::new(seed);
    for i in (1..items.len()).rev() {
        let j = rng.below(i + 1);
        items.swap(i, j);
    }
}

/// Semilla sin I/O externa: reloj, pid, dirección de pila y un contador
/// atómico que garantiza valores distintos incluso dentro del mismo nanosegundo.
fn entropy_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

    let counter = COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    let marker = 0u8;
    let stack = (&marker as *const u8 as usize) as u64;

    counter ^ nanos.rotate_left(17) ^ pid.rotate_left(32) ^ stack
}

/// Generador SplitMix64: pequeño, rápido y suficiente para barajar índices.
#[derive(Debug, Clone)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        debug_assert!(bound > 0);
        (self.next_u64() % bound as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeats() -> [RepeatMode; 3] {
        [RepeatMode::Off, RepeatMode::All, RepeatMode::One]
    }

    fn sorted(mut values: Vec<usize>) -> Vec<usize> {
        values.sort_unstable();
        values
    }

    fn assert_permutation(order: &[usize], len: usize) {
        assert_eq!(order.len(), len);
        assert_eq!(sorted(order.to_vec()), (0..len).collect::<Vec<_>>());
    }

    fn starts(len: usize) -> Vec<usize> {
        if len == 0 {
            vec![0]
        } else {
            (0..len).collect()
        }
    }

    #[test]
    fn repeat_mode_cycles_and_types_derive() {
        assert_eq!(RepeatMode::default(), RepeatMode::Off);
        assert_eq!(RepeatMode::Off.next(), RepeatMode::All);
        assert_eq!(RepeatMode::All.next(), RepeatMode::One);
        assert_eq!(RepeatMode::One.next(), RepeatMode::Off);

        assert_eq!(Advance::To(1), Advance::To(1));
        assert_ne!(Advance::Stop, Advance::To(0));
        assert_eq!(Direction::Next, Direction::Next);
        assert_ne!(Direction::Next, Direction::Prev);
        assert_eq!(MoveOutcome::To(1), MoveOutcome::To(1));
        assert_ne!(MoveOutcome::Stop, MoveOutcome::To(0));
    }

    #[test]
    fn advance_index_respects_repeat_and_direction() {
        assert_eq!(advance_index(0, 0, RepeatMode::All, Direction::Next), None);
        assert_eq!(advance_index(0, 0, RepeatMode::All, Direction::Prev), None);

        for repeat in repeats() {
            assert_eq!(advance_index(5, 10, repeat, Direction::Next), Some(6));
            assert_eq!(advance_index(5, 10, repeat, Direction::Prev), Some(4));
        }

        assert_eq!(advance_index(9, 10, RepeatMode::Off, Direction::Next), None);
        assert_eq!(advance_index(9, 10, RepeatMode::One, Direction::Next), None);
        assert_eq!(
            advance_index(9, 10, RepeatMode::All, Direction::Next),
            Some(0)
        );

        assert_eq!(advance_index(0, 10, RepeatMode::Off, Direction::Prev), None);
        assert_eq!(advance_index(0, 10, RepeatMode::One, Direction::Prev), None);
        assert_eq!(
            advance_index(0, 10, RepeatMode::All, Direction::Prev),
            Some(9)
        );

        // Una sola pista: solo `All` puede "moverse" (hacia sí misma).
        assert_eq!(advance_index(0, 1, RepeatMode::Off, Direction::Next), None);
        assert_eq!(advance_index(0, 1, RepeatMode::One, Direction::Prev), None);
        assert_eq!(
            advance_index(0, 1, RepeatMode::All, Direction::Next),
            Some(0)
        );
        assert_eq!(
            advance_index(0, 1, RepeatMode::All, Direction::Prev),
            Some(0)
        );

        // Un cursor fuera de rango se satura al último índice válido.
        assert_eq!(
            advance_index(99, 10, RepeatMode::Off, Direction::Prev),
            Some(8)
        );
        assert_eq!(
            advance_index(99, 10, RepeatMode::Off, Direction::Next),
            None
        );
        assert_eq!(
            advance_index(99, 10, RepeatMode::All, Direction::Next),
            Some(0)
        );
    }

    #[test]
    fn auto_and_manual_advance_apply_their_own_policy() {
        assert_eq!(auto_advance(0, 0, RepeatMode::One), Advance::Stop);
        assert_eq!(auto_advance(9, 10, RepeatMode::One), Advance::RepeatCurrent);
        assert_eq!(auto_advance(9, 10, RepeatMode::Off), Advance::Stop);
        assert_eq!(auto_advance(9, 10, RepeatMode::All), Advance::To(0));
        assert_eq!(auto_advance(0, 1, RepeatMode::Off), Advance::Stop);
        assert_eq!(auto_advance(0, 1, RepeatMode::All), Advance::To(0));

        // En manual, `One` no repite: el usuario pidió avanzar.
        assert_eq!(manual_next(9, 10, RepeatMode::One), Advance::Stop);
        assert_eq!(manual_next(9, 10, RepeatMode::Off), Advance::Stop);
        assert_eq!(manual_next(9, 10, RepeatMode::All), Advance::To(0));
        assert_eq!(manual_next(0, 10, RepeatMode::One), Advance::To(1));
        assert_eq!(manual_next(0, 10, RepeatMode::Off), Advance::To(1));
        assert_eq!(manual_next(0, 1, RepeatMode::One), Advance::Stop);
        assert_eq!(manual_next(0, 0, RepeatMode::All), Advance::Stop);
    }

    #[test]
    fn advance_walks_hundred_steps_without_panicking() {
        for len in [0usize, 1, 2, 10] {
            for repeat in repeats() {
                for &start in &starts(len) {
                    let mut current = start;
                    for step in 0..100 {
                        let direction = if step % 2 == 0 {
                            Direction::Next
                        } else {
                            Direction::Prev
                        };
                        if let Some(next) = advance_index(current, len, repeat, direction) {
                            assert!(next < len);
                            current = next;
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn shuffle_new_puts_current_first_without_duplicates() {
        for len in [0usize, 1, 2, 10] {
            for current in starts(len) {
                let state = ShuffleState::new(len, current);
                assert!(!state.is_enabled());
                assert_permutation(state.order_snapshot(), len);
                if len > 0 {
                    assert_eq!(state.order_snapshot()[0], current);
                }
            }
        }

        // Un índice fuera de rango se recorta sin romper la permutación.
        let state = ShuffleState::new(4, 99);
        assert_permutation(state.order_snapshot(), 4);
        assert_eq!(state.order_snapshot()[0], 3);
    }

    #[test]
    fn shuffle_is_reproducible_with_a_fixed_seed() {
        let mut first: Vec<usize> = (0..10).collect();
        let mut second: Vec<usize> = (0..10).collect();
        let seed = 0x1234_5678_9ABC_DEF0;

        shuffle_in_place(&mut first, seed);
        shuffle_in_place(&mut second, seed);

        assert_eq!(first, second);
        assert_permutation(&first, 10);
        // No se exige una permutación concreta: el azar podría dejar el orden
        // original. Solo importa que no se pierdan ni dupliquen índices.
    }

    #[test]
    fn shuffle_enable_is_idempotent_and_keeps_history() {
        let mut state = ShuffleState::new(10, 3);
        state.enable(10, 3);
        assert!(state.is_enabled());

        let first_order = state.order_snapshot().to_vec();
        assert!(matches!(state.next(10, RepeatMode::Off), Advance::To(_)));

        // Volver a activar no debe barajar de nuevo ni borrar el historial.
        state.enable(10, 9);
        assert!(state.is_enabled());
        assert_eq!(state.order_snapshot(), &first_order[..]);
        assert_eq!(state.prev(10), Some(3));

        // También es idempotente con la cola vacía.
        let mut empty = ShuffleState::new(0, 0);
        empty.enable(0, 0);
        empty.enable(0, 0);
        assert!(empty.is_enabled());
        assert!(empty.order_snapshot().is_empty());
    }

    #[test]
    fn shuffle_disable_restores_current_and_is_idempotent() {
        for len in [0usize, 1, 2, 10] {
            let mut state = ShuffleState::new(len, 0);
            state.enable(len, 0);
            let mut current = state.order_snapshot().first().copied().unwrap_or(0);

            for _ in 0..3 {
                match state.next(len, RepeatMode::Off) {
                    Advance::To(index) => current = index,
                    Advance::Stop => break,
                    Advance::RepeatCurrent => panic!("ShuffleState no repite directamente"),
                }
            }

            let order_before = state.order_snapshot().to_vec();
            if len > 0 {
                assert!(order_before.contains(&current));
            }

            assert_eq!(state.disable(current), current);
            assert!(!state.is_enabled());
            assert_eq!(state.disable(current), current);
            assert!(!state.is_enabled());
            assert_eq!(state.order_snapshot(), &order_before[..]);
            assert_permutation(state.order_snapshot(), len);

            // Al reactivar, la pista actual vuelve a encabezar la baraja.
            state.enable(len, current);
            assert!(state.is_enabled());
            assert_permutation(state.order_snapshot(), len);
            if len > 0 {
                assert_eq!(state.order_snapshot()[0], current.min(len - 1));
            }
        }
    }

    #[test]
    fn shuffle_next_off_walks_the_order_and_prev_unwinds_history() {
        for len in [1usize, 2, 10] {
            for current in 0..len {
                let mut state = ShuffleState::new(len, current);
                state.enable(len, current);
                let expected = state.order_snapshot().to_vec();

                for &track in &expected[1..] {
                    assert_eq!(state.next(len, RepeatMode::Off), Advance::To(track));
                }
                assert_eq!(state.next(len, RepeatMode::Off), Advance::Stop);
                // Un Stop repetido no debe mover el cursor ni el historial.
                assert_eq!(state.next(len, RepeatMode::Off), Advance::Stop);

                let mut back = Vec::new();
                while let Some(previous) = state.prev(len) {
                    back.push(previous);
                }
                let expected_back: Vec<usize> = expected[..expected.len() - 1]
                    .iter()
                    .rev()
                    .copied()
                    .collect();
                assert_eq!(back, expected_back);
                assert_eq!(state.prev(len), None);
            }
        }
    }

    #[test]
    fn shuffle_repeat_all_starts_a_new_cycle_without_immediate_repeat() {
        let len = 10;
        let mut state = ShuffleState::new(len, 4);
        state.enable(len, 4);

        let mut last = 4;
        for _ in 1..len {
            match state.next(len, RepeatMode::All) {
                Advance::To(index) => {
                    assert_ne!(index, last);
                    last = index;
                }
                other => panic!("All no debería parar: {other:?}"),
            }
        }

        // Estábamos en la última pista del ciclo. `All` rebaraja y devuelve
        // otra pista, dejando la anterior al frente de la nueva baraja.
        let Advance::To(first_new) = state.next(len, RepeatMode::All) else {
            panic!("All debe iniciar un ciclo nuevo");
        };
        assert_ne!(first_new, last);
        assert_permutation(state.order_snapshot(), len);
        assert_eq!(state.order_snapshot()[0], last);

        // Quedan exactamente las otras `len - 2` pistas del ciclo nuevo.
        let mut seen = vec![first_new];
        for _ in 0..len - 2 {
            let Advance::To(index) = state.next(len, RepeatMode::All) else {
                panic!("el ciclo nuevo no debe agotarse antes de tiempo");
            };
            seen.push(index);
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), len - 1);
        assert!(!seen.contains(&last));
    }

    #[test]
    fn shuffle_next_treats_one_as_stop_at_the_end() {
        let mut state = ShuffleState::new(2, 0);
        state.enable(2, 0);
        assert!(matches!(state.next(2, RepeatMode::One), Advance::To(_)));
        assert_eq!(state.next(2, RepeatMode::One), Advance::Stop);
        assert_eq!(state.next(2, RepeatMode::One), Advance::Stop);
    }

    #[test]
    fn shuffle_repeat_all_single_track_repeats_it() {
        let mut state = ShuffleState::new(1, 0);
        state.enable(1, 0);
        for _ in 0..5 {
            assert_eq!(state.next(1, RepeatMode::All), Advance::To(0));
        }
        assert_eq!(state.next(1, RepeatMode::Off), Advance::Stop);
        assert_eq!(state.next(1, RepeatMode::One), Advance::Stop);
    }

    #[test]
    fn shuffle_walks_hundred_steps_without_panicking() {
        for len in [0usize, 1, 2, 10] {
            for repeat in repeats() {
                let mut state = ShuffleState::new(len, 0);
                state.enable(len, 0);

                for step in 0..100 {
                    if step % 3 == 2 {
                        if let Some(previous) = state.prev(len) {
                            assert!(previous < len);
                        }
                    } else {
                        match state.next(len, repeat) {
                            Advance::To(index) => assert!(index < len),
                            Advance::RepeatCurrent => {
                                panic!("ShuffleState nunca devuelve RepeatCurrent")
                            }
                            Advance::Stop => {
                                assert!(len == 0 || repeat != RepeatMode::All);
                            }
                        }
                    }
                    assert_permutation(state.order_snapshot(), len);
                }
            }
        }
    }

    #[test]
    fn shuffle_on_len_changed_rebuilds_and_clears_history() {
        let mut state = ShuffleState::new(5, 2);
        state.enable(5, 2);
        assert!(matches!(state.next(5, RepeatMode::Off), Advance::To(_)));

        state.on_len_changed(8, 6);
        assert!(state.is_enabled());
        assert_permutation(state.order_snapshot(), 8);
        assert_eq!(state.order_snapshot()[0], 6);
        assert_eq!(state.prev(8), None, "el backup anterior queda invalidado");

        // Encogimiento con índice fuera de rango: se recorta.
        state.on_len_changed(2, 9);
        assert_permutation(state.order_snapshot(), 2);
        assert_eq!(state.order_snapshot()[0], 1);

        // Vaciado total.
        state.on_len_changed(0, 0);
        assert!(state.is_enabled());
        assert!(state.order_snapshot().is_empty());
        assert_eq!(state.next(0, RepeatMode::All), Advance::Stop);
        assert_eq!(state.prev(0), None);

        // El flag no se toca cuando estaba desactivado.
        let mut disabled = ShuffleState::new(3, 0);
        assert!(!disabled.is_enabled());
        disabled.on_len_changed(6, 5);
        assert!(!disabled.is_enabled());
        assert_eq!(disabled.order_snapshot()[0], 5);
        assert_permutation(disabled.order_snapshot(), 6);
    }

    #[test]
    fn shuffle_repairs_length_mismatch_on_next() {
        let mut state = ShuffleState::new(3, 1);
        state.enable(3, 1);

        // Mutación externa sin llamar a `on_len_changed`.
        assert!(matches!(state.next(5, RepeatMode::Off), Advance::To(_)));
        assert_permutation(state.order_snapshot(), 5);

        if let Some(previous) = state.prev(5) {
            assert!(previous < 5);
        }
    }

    #[test]
    fn remove_index_adjusts_cursor() {
        // Borrar antes, después y la actual.
        assert_eq!(remove_index(2, 0, 5), Some(1));
        assert_eq!(remove_index(2, 3, 5), Some(2));
        assert_eq!(remove_index(2, 2, 5), Some(2));
        assert_eq!(remove_index(4, 4, 5), None);
        assert_eq!(remove_index(0, 0, 1), None);

        // Inválidos.
        assert_eq!(remove_index(0, 0, 0), None);
        assert_eq!(remove_index(5, 0, 5), None);
        assert_eq!(remove_index(0, 5, 5), None);

        // Exhaustivo contra el comportamiento real de `Vec::remove`.
        for len in 1..=6 {
            for current in 0..len {
                for index in 0..len {
                    let mut tracks: Vec<usize> = (0..len).collect();
                    tracks.remove(index);
                    let expected = if index == current {
                        if current < tracks.len() {
                            Some(current)
                        } else {
                            None
                        }
                    } else {
                        Some(tracks.iter().position(|&track| track == current).unwrap())
                    };
                    assert_eq!(
                        remove_index(current, index, len),
                        expected,
                        "len={len} current={current} index={index}"
                    );
                }
            }
        }
    }

    #[test]
    fn move_index_adjusts_cursor() {
        // Mover una entrada de antes a después de la actual.
        assert_eq!(move_index(2, 0, 3, 5), Some(1));
        // Mover una entrada de después a antes de la actual.
        assert_eq!(move_index(2, 4, 1, 5), Some(3));
        // Mover la propia pista actual.
        assert_eq!(move_index(2, 2, 0, 5), Some(0));
        assert_eq!(move_index(2, 2, 4, 5), Some(4));
        // Movimientos que no cruzan el cursor.
        assert_eq!(move_index(2, 1, 1, 5), Some(2));
        assert_eq!(move_index(2, 3, 4, 5), Some(2));

        // Inválidos.
        assert_eq!(move_index(0, 0, 0, 0), None);
        assert_eq!(move_index(5, 0, 1, 5), None);
        assert_eq!(move_index(0, 5, 1, 5), None);
        assert_eq!(move_index(0, 0, 5, 5), None);

        // Exhaustivo contra `Vec::remove` + `Vec::insert`.
        for len in 1..=6 {
            for current in 0..len {
                for from in 0..len {
                    for to in 0..len {
                        let mut tracks: Vec<usize> = (0..len).collect();
                        let moved = tracks.remove(from);
                        tracks.insert(to, moved);
                        let expected = tracks.iter().position(|&track| track == current).unwrap();
                        assert_eq!(
                            move_index(current, from, to, len),
                            Some(expected),
                            "len={len} current={current} from={from} to={to}"
                        );
                    }
                }
            }
        }
    }
}
