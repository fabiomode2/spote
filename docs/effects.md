# Efectos visuales: port de `cli-viz` a Rust/ratatui

Especificación de implementación. Fuente: `sam1am/cli-viz` @ `e1c449d`
(`/tmp/cli-viz`), 4011 líneas Python repartidas en `main.py`,
`visualizer_base.py` y 15 ficheros de `visualizers/`. Destino: los efectos de
visualización de `spote` (§7.4 de `idea.md`).

Esto **no** es un port de código: cli-viz dibuja con `curses` sobre la pantalla
entera, sin composición, con HSV aleatorio y sin control de coste. Aquí se porta
el **mecanismo** (spawn rates, decaimientos, difusión, proyecciones) a un trait
común, con la paleta del álbum, presupuesto de CPU y composición por capas.

> **No se toca nada más del repo.** Este documento es el único artefacto.

---

## Índice

0. [Resumen de ficheros](#0-resumen-de-ficheros)
1. [Contrato común: `AudioFrame`, `VizCtx`, `Layer`](#1-contrato-común-audioframe-vizctx-layer)
   - 1.0 Estado del código y diferencias con este documento
   - 1.1 De cli-viz a `AudioFrame`
   - 1.2 Mapeo de índices lineales (1024 bins) a 64 bandas log
   - 1.3 `energy`: calibración y tabla de umbrales
   - 1.4 Reglas de oro del puerto (tiempo, RNG, aspect, allocs)
   - 1.5 Equivalencias numpy → Rust
2. [Color: de HSV a la rampa del álbum](#2-color-de-hsv-a-la-rampa-del-álbum)
3. [Los efectos](#3-los-efectos)
   - 3.1 bars · 3.2 wave · 3.3 circle · 3.4 particles · 3.5 flame
   - 3.6 matrix · 3.7 starfield · 3.8 fractal · 3.9 neural · 3.10 neural-lite
   - 3.11 pulsar · 3.12 stick · 3.13 tuner
4. [Composición por capas y blend modes](#4-composición-por-capas-y-blend-modes)
5. [Prioridad de implementación](#5-prioridad-de-implementación)
6. [Qué copiar de spotatui (y qué no)](#6-qué-copiar-de-spotatui-y-qué-no)
7. [Anexos: glifos y checklist](#7-anexos)

---

## 0. Resumen de ficheros

| Fichero | LOC | Método que importa | Notas |
|---|---|---|---|
| `main.py` | 205 | `get_audio_data` L79-97, `setup_colors` L99-128, `run` L130-201 | FFT, normalización, smoothing, `energy`, `hue_offset` |
| `visualizer_base.py` | 93 | `get_color_pair` L40-67, `hsv_to_color_pair` L69-93 | cubo 6×6×6 de curses |
| `visualizers/bars.py` | 41 | `draw` L11-33 | barras, boost de graves |
| `visualizers/wave.py` | 38 | `draw` L11-36 | suma de senos |
| `visualizers/circle.py` | 39 | `draw` L10-35 | anillos |
| `visualizers/particles.py` | 65 | `draw` L12-57 | física de partículas |
| `visualizers/flame.py` | 149 | `draw` L20-131 | buffer de calor + difusión |
| `visualizers/matrix_rain.py` | 181 | `spawn_drops` L27-48, `draw` L50-153 | columnas con cola |
| `visualizers/starfield_warp.py` | 278 | `draw` L31-258 | proyección 3D + ondas |
| `visualizers/fractal_universe.py` | 326 | `compute_fractal` L46-171, `draw` L173-283 | Mandelbrot/Julia |
| `visualizers/fractal_universe_lite.py` | 274 | `compute_fractal` L41-136 | downsample 2× + skip de frames |
| `visualizers/neural_dreamscape.py` | 509 | `update_neural_field` L97-318, `draw` L319-509 | 3 campos full-screen |
| `visualizers/neural_dreamscape_lite.py` | 862 | `update_neural_field` L180-525, `get_display_color` L527-550, `draw` L552-862 | downsample 2×, neuronas tipadas |
| `visualizers/cosmic_pulsar.py` | 388 | `_get_freq_bands` L49-69, `draw` L71-357 | el más caro y el más vistoso |
| `visualizers/stick_figure.py` | 183 | `draw` L16-146, `draw_line` L148-172 | Bresenham |
| `visualizers/guitar_tuner.py` | 348 | `get_string_frequency` L31-99, `draw` L101-326 | widget, no efecto |
| `visualizers/README.md` | 6.4 KB | — | contrato de plugins (irrelevante para el port) |

**Lo que se descarta del original, sin discusión:**

- `curses.color_pair` / cubo 216 colores. Sustituido por RGB directo de la
  paleta (§2).
- `print()` de `load_visualizers` y las barras de debug que cada efecto dibuja
  en la última fila (`matrix` L136-138, `starfield` L252-259, `fractal` L185-187
  y L261-269, `neural` L334-337). En spote eso lo pinta el footer.
- Las teclas propias de cada efecto (`b/B`, `w/W`, `h/H`, `d/D`, `s/S`, `l/L`,
  `c`, `t/T`, `p/P`, `n/N`, `i/I`, `j`, `r`, `1..3`, `9`, `0`). Colisionan entre
  efectos y con el mapa global de §7.3 de `idea.md`. Los parámetros van a un
  panel de ajustes del efecto activo; el teclado global solo cicla efectos
  (`1-9`, `v`, `V`) y presets (`S`).
- `time.time()` y `random.*` globales: ver §1.4.

---

## 1. Contrato común: `AudioFrame`, `VizCtx`, `Layer`

### 1.0 Estado del código y diferencias con este documento

Cuando se escribió esto ya había implementación en marcha:
`crates/core/src/audio.rs` (`AudioFrame`, `BANDS = 64`, `WAVE = 256`),
`crates/tui/src/viz/mod.rs` (`Cell`, `Layer`, `Blend`, `Mask`, `Params`,
`VizCtx`, `Effect`, `Rng`, `PRESETS`) y `crates/tui/src/viz/effects.rs` (12
efectos: `bars, wave, circle, particles, flame, matrix, starfield, fractal,
neural, pulsar, tunnel, spectro3d`; el `pulsar`/`tunnel`/`spectro3d` del código
no son los mismos ficheros que los de cli-viz).

**Donde el código ya resuelve algo, manda el código.** Las diferencias que
importan al portar lo que falta:

| Este documento | Código | Comentario |
|---|---|---|
| `AudioFrame { …, hue, palette }` | `AudioFrame { bands, waveform, bass, mid, treble, energy, beat, beat_pulse, time, silent }` | el tono global y la paleta van en `VizCtx` (§2). `silent` ahorra el `energy < eps` repartido |
| `VizCell { glyph: u8, ink: u8, fg }` (8 B) | `Cell { ch: char, fg: Rgb, alpha: f32 }` (16 B) | `char` evita el LUT de glifos a cambio de no validar el ancho: ver Anexo A |
| `LayerBuf::set` | `Layer::put(x, y, ch, fg, alpha)` | `put` recorta y resuelve el glifo por **"mayor alpha gana"**; §4.3 razona por qué conviene resolverlo por tinta |
| `Mask` de 6 variantes | `Mask { Full, Radial(power), Bottom, Band(t0, t1) }` | `Band` es una franja **vertical del canvas**, no una banda de frecuencias: la máscara de espectro de §4.4 **no existe** todavía |
| 5 presets (§4.7) | `PRESETS`: `aurora, inferno, void, bloom, glitch, clean, neural` | mismos nombres para los cinco primeros, capas distintas: ver la nota al final de §4.7 |
| `Params { gain, … }` por efecto | `Params { sensitivity, density, speed }` | `sensitivity` hace de `gain` |

El resto del documento (algoritmos, fórmulas, costes, calibración y blend) aplica
igual.

Lo que cli-viz le pasa a cada `draw`:

```python
draw(stdscr, spectrum, height, width, energy, hue_offset)
#  spectrum : np.ndarray float64, 1024 bins lineales, 21.53 Hz/bin, 0..~128
#  height, width : tamaño de la PANTALLA COMPLETA (no de un rect)
#  energy   : float, típicamente 0.00..0.50
#  hue_offset : float 0..1, avanza 0.005 por frame
```

Lo que necesita el puerto (de `idea.md` §7.4, con dos añadidos):

```rust
pub struct AudioFrame {
    pub bands: [f32; 64],      // log 40 Hz..16 kHz, 0..1, con attack/release
    pub bass: f32,             // agregados ya calculados (ver 1.3)
    pub mid: f32,
    pub treble: f32,
    pub energy: f32,           // RMS normalizado 0..1
    pub beat: bool,            // onset
    pub beat_pulse: f32,       // 0..1 con decay
    pub waveform: [f32; 256],  // osciloscopio -1..1
    pub hue: f32,              // fase lenta global (= hue_offset de cli-viz)
    pub time: f32,             // segundos desde el arranque del efecto
    pub palette: Palette,
    // --- añadidos necesarios para el port fiel ---
    pub flux: f32,             // 0..1, suma de |dB/dt| normalizada (onset suave)
    pub peak: [f32; 64],       // pico con hold+gravedad para bars (opcional)
}

pub struct VizCtx {
    pub rect: Rect,            // zona asignada al efecto, ya recortada
    pub dt: f32,               // segundos del frame (clamp 0.001..0.100)
    pub fps: f32,
    pub paused: bool,
    pub frame: u64,            // contador monótono, para hashes y animaciones
    pub behind_art: bool,      // true si la capa va detrás del arte ASCII
}
```

`hue_offset` avanza `0.005` por frame a 60 fps = `0.3/s`, o sea un ciclo de color
completo cada 3.3 s. **En el puerto esto no existe**: el "color que gira" se
sustituye por (a) la rampa de luminancia del álbum y (b) un ciclo lento opcional
de `primary↔secondary` de 8 s para los efectos que se veían "arcoíris"
(matrix, pulsar). Ver §2.

### 1.1 De cli-viz a `AudioFrame`

`main.py` L79-97, paso a paso:

```python
CHUNK = 2048; RATE = 44100
data = frombuffer(read(CHUNK), int16)              # mono
spectrum = abs(fft(data)[:1024])                   # magnitud, 1024 bins
spectrum = spectrum / (128 * CHUNK)                 # = /262144
smoothed = smoothed*0.8 + spectrum*0.2              # suavizado exponencial
adjusted = smoothed * sensitivity                   # sensitivity 1.0, +/-0.1
energy   = mean(adjusted[:512]) * 2                 # L116
```

Tres consecuencias que hay que tener presentes al calibrar:

1. **Un bin suelto puede valer ~100**, pero la **media** de 512 bins casi
   siempre está por debajo de 0.5, porque la mayoría de los bins están a cero.
   De ahí que cli-viz lea bins directos sin ganancia (`bars`) pero multiplique
   las medias por 2, 3, 5, 10 o 15 (`flame`, `pulsar`, `starfield`). En spote
   las 64 bandas ya vienen 0..1, así que **esas ganancias sobran**: se sustituyen
   por un `gain` de parámetro con default 1.0 (§1.3).
2. `energy` en cli-viz es una **media de magnitudes**, no RMS: sube con el
   número de parciales, no con el volumen percibido. La `energy` de spote es
   RMS. La correlación es monótona pero la escala no: ver la tabla de §1.3.
3. El suavizado es un one-pole de 0.8 por frame (`coef = 0.8`). spote ya aplica
   attack/release en el thread de audio; no se vuelve a suavizar el array de
   bandas. Lo que sí se suaviza **por efecto** son las magnitudes internas
   (resonance, beat_pulse, needle, etc.), con el mismo one-pole pero pasado a
   tiempo real: `x = x*exp(-dt/tau) + target*(1-exp(-dt/tau))`.

Resolución: `bin_width = 44100/2048 = 21.533 Hz`. El bin 1 es 21.5 Hz, el 10 son
215 Hz, el 30 son 646 Hz, el 512 son 11.0 kHz. **Los bins 0..1 no tienen
resolución para nada por debajo de ~43 Hz**, así que las primeras 3-4 bandas log
de spote (40..50 Hz) se calculan con 0 o 1 bin de FFT y son ruidosas: se
interpolan desde la banda 4 y se marcan como "no medidas" (§1.2).

### 1.2 Mapeo de índices lineales (1024 bins) a 64 bandas log

Las bandas de spote: `f(k) = 40 * 400^(k/64)`, es decir **1.0981× por banda**,
~162 cents, **7.4 bandas por octava** (64 bandas = 8.64 octavas = log2(16000/40)). `k=0` → 40 Hz, `k=32` → 800 Hz, `k=64` →
16 kHz. Tabla de referencia:

| k | Hz | k | Hz | k | Hz | k | Hz |
|---|---|---|---|---|---|---|---|
| 0 | 40 | 16 | 179 | 32 | 800 | 48 | 3 577 |
| 4 | 57 | 20 | 260 | 36 | 1 065 | 52 | 5 202 |
| 8 | 83 | 24 | 378 | 40 | 1 692 | 56 | 7 571 |
| 12 | 126 | 28 | 550 | 44 | 2 460 | 60 | 10 900 |
| 14 | 152 | 30 | 665 | 46 | 2 968 | 63 | 14 580 |

**Tabla de traducción de los slices de cli-viz** (esto es lo que se consulta en
cada efecto; no repetir el cálculo a mano):

| Slice en cli-viz | Bins | Hz reales | Bandas en spote |
|---|---|---|---|
| `spectrum[:5]` | 1-4 | 22-86 | `bands[0..9]` (40-96 Hz) |
| `spectrum[:6]` | 1-5 | 22-108 | `bands[0..10]` |
| `spectrum[:10]` | 1-9 | 22-194 | `bands[0..18]` (40-215 Hz) |
| `spectrum[10:12]` | 10-11 | 215-237 | `bands[18..20]` |
| `spectrum[5:15]` | 5-14 | 108-301 | `bands[10..20]` |
| `spectrum[10:20]` | 10-19 | 215-409 | `bands[18..24]` |
| `spectrum[10:30]` | 10-29 | 215-624 | `bands[18..30]` (215-665 Hz) |
| `spectrum[12:20]` | 12-19 | 258-409 | `bands[20..24]` |
| `spectrum[:20]` | 1-19 | 22-409 | `bands[0..24]` |
| `spectrum[20:]` | 20-1023 | 430-22 k | `bands[24..64]` |
| `spectrum[30:]` | 30-1023 | 646-22 k | `bands[30..64]` (665 Hz-16 k) |
| `spectrum[:102]` (pulsar) | 0-101 | 0-2 175 | `bands[0..50]` → **no usar**, ver 3.11 |
| `spectrum[:341]` (pulsar) | 0-340 | 0-7 325 | `bands[0..62]` → **no usar** |

Regla general: `bass = bands[0..18]`, `lowmid = bands[18..30]`,
`mid = bands[30..44]`, `high = bands[44..56]`, `treble = bands[56..64]`.

Para leer **un** índice concreto (bars, matrix, pulsar) la traducción es
`k = 0` en el bin 1..2, y el mapeo inverso
`bin ≈ 21.533 * 40 * 1.0981^k / 21.533 → bin ≈ 1.86 * 1.0981^k`:

```rust
// equivalente a spectrum[i] en cli-viz, para i pequeño (0..160)
fn band_for_bin(i: usize) -> usize {
    // bin i = i*21.533 Hz -> k = 64*ln(i*21.533/40)/ln(400)
    if i == 0 { return 0; }
    let k = 64.0 * ((i as f32 * 21.533) / 40.0).ln() / 400f32.ln();
    (k.round() as usize).clamp(0, 63)
}
```

Ojo: por debajo del bin 2 (43 Hz) y por encima del 743 (16 kHz) no hay
correspondencia. La banda 63 cubre los bins 677..743 (14.6-16.0 kHz) y suele
venir casi vacía con códec lossy; los bins 744..1023 (16.0-22.05 kHz) no se usan
para nada.

Y el agregado de un rango, que aparece en todos los efectos:

```rust
fn mean(a: &AudioFrame, lo: usize, hi: usize) -> f32 {
    let s: f32 = a.bands[lo..hi].iter().sum();
    s / (hi - lo) as f32
}
```

### 1.3 `energy`: calibración y tabla de umbrales

La `energy` de cli-viz y la de spote no comparten escala. Para no re-tunear cada
efecto a ciegas, este es el mapeo de partida (a validar con una pista real, no
con un tono puro):

| Condición en cli-viz | Significado | En spote |
|---|---|---|
| `energy > 0.01` | hay audio | `energy > 0.02` |
| `energy > 0.05` | suena algo | `energy > 0.04` |
| `energy > 0.1` | sonido claro | `energy > 0.08 \|\| beat_pulse > 0.2` |
| `energy > 0.2` | "beat" | `beat \|\| energy > 0.20` |
| `energy > 0.25` | beat fuerte | `beat && energy > 0.25` |
| `energy > 0.3` (pulsar) | beat para anillos | `beat \|\| (flux > 0.35 && energy > 0.22)` |
| `energy > 0.4` | beat muy fuerte | `beat && (bass > 0.55 \|\| energy > 0.45)` |
| `int(energy * 10)` | nº de partículas | `(beat_pulse * 10.0) as usize` |
| `energy * 15` | spawn de matrix | `(energy * 8.0) as usize + 4*beat as usize` |

Y los agregados, calculados **una vez** por frame en el productor de
`AudioFrame` (no dentro de cada efecto):

```rust
let bass   = mean(&a, 0, 16);   // 40-170 Hz: bombo, bajo
let lowmid = mean(&a, 16, 28);  // 170-520 Hz: cuerpo
let mid    = mean(&a, 28, 44);  // 520-2.4k: voz, caja
let high   = mean(&a, 44, 56);  // 2.4-7.4k: presencia, hats
let treble = mean(&a, 56, 64);  // 7.4-16k: aire
```

Cada efecto expone `gain: f32` (0.2..4.0, default el de su tabla) y se calibra
con dos comprobaciones:

1. **Pista con bombo claro** (techno, hip-hop): a `gain` default, `bass` debe
   oscilar 0.4..0.95 con el bombo y bajar a ~0.25 entre golpes. Si está siempre
   a 1.0, baja `gain`; si no se mueve, súbelo.
2. **Piano solo / acústico**: `bass` ~0.1-0.3, `mid` 0.3-0.7, `treble` 0.05-0.2.
   Un efecto que parezca muerto con esto está mal calibrado, no es la pista.

### 1.4 Reglas de oro del puerto

1. **Tiempo real, nunca frames.** Todos los originales son por-frame a ~60 fps
   (`main.py` L203: `time.sleep(0.016)`). Toda constante de velocidad se escribe
   como `x += v * dt * 60.0` con `dt` en segundos. Sin esto, a 30 fps la llama
   sube la mitad de rápido y a 144 fps se dispara.
2. **RNG propio por efecto, sin locks.** `SmallRng` (`rand 0.9`), sembrado con el
   id de la pista + el índice del efecto, guardado en el struct. Nunca
   `thread_rng()` dentro del render (toma un lock por llamada) ni `rand::random`.
   `random.uniform(a,b)` → `rng.random_range(a..=b)` para f32 o
   `a + rng.random::<f32>() * (b - a)`; `random.random()` → `rng.random::<f32>()`.
   Regla: **como mucho 64 llamadas al RNG por frame**. Donde el original pide
   uno por celda (flame pide `fw*fh` = 2400), se sustituye por un hash barato de
   `(x, y, frame)` (ver 3.5).
3. **Aspecto de celda.** Una celda de terminal mide ~1:2 (alto:ancho). cli-viz
   ignora esto y sus "círculos" son elipses achatadas. En el puerto, todo lo
   radial usa `const CELL_ASPECT: f32 = 2.0` y multiplica la componente X:
   `x = cx + r * cos(θ) * CELL_ASPECT`. Es un parámetro de gusto (2.0 en
   monospace normal, 1.8-2.2 según la fuente).
4. **Cero allocs por frame.** Buffers `Vec<f32>` y `Vec<VizCell>` dimensionados
   en `resize()`; `fill(0.0)` en vez de `vec![0.0; n]`. Nada de `format!`,
   `String`, `Vec::new()` ni `Box` dentro de `render`. Nada de iteradores que
   reserven. Un `Vec` que crece (partículas, drops) se declara con
   `with_capacity(max)` y crece como mucho una vez.
5. **`f32` en todo** salvo el `log` del fractal suavizado (usar `f64` solo ahí).
6. **Recorte, no comprobación por celda.** Se dibuja en un buffer local de
   `rect.w * rect.h` y se compone con el resto; así desaparecen los cientos de
   `if 0 <= x < width` del original (y los `try/except curses.error`, que en
   Python son el 20 % de las líneas de algunos efectos).
7. **Nada de emoji ni CJK.** Ver §7 Anexo A.

### 1.5 Equivalencias numpy → Rust

| numpy / Python | Rust |
|---|---|
| `np.mean(spectrum[a:b])` | `slice.iter().sum::<f32>() / n as f32` (nunca 0) |
| `np.zeros((h,w))` | `vec![0.0f32; h*w]`, indexado `y*w + x` |
| `np.clip(a, 0, 1)` | `a.clamp(0.0, 1.0)` |
| `np.zeros_like(f)` + doble bucle | segundo `Vec<f32>` persistente + `std::mem::swap` |
| `arr *= 0.9` (broadcast) | `for v in buf.iter_mut() { *v *= 0.9; }` (o `fill` si es constante) |
| `complex` + `abs(z)` | `(re: f32, im: f32)`; `abs` = `(re*re+im*im).sqrt()`, y **comparar al cuadrado** contra `r²` para ahorrar la raíz |
| `cmath.exp(complex(0,a))` | `(a.cos(), a.sin())` |
| `z*z + c` | `(re*re - im*im + cr, 2.0*re*im + ci)` |
| `random.choice(list)` | `list[rng.random_range(0..list.len())]` |
| `list.pop(0)` | `VecDeque` o buffer circular de tamaño fijo |
| `dict[(y,x)]` (neuronas) | `Vec<u8>` máscara `w*h` + arrays paralelos (ver 3.9) |
| `math.atan2(dy,dx)` | `dy.atan2(dx)` |
| `int(x)` (truncar hacia 0) | `x as i32 as u16` ojo con negativos: usar `x.round() as i32` y recortar |
| iterar 360° cada 5° | `for i in 0..72 { let a = i as f32 * TAU / 72.0; }` con LUT de seno/coseno |

**LUT de trigonometría.** Varios efectos piden `sin/cos` por celda (máscaras
radiales, anillos, fractal con rotación, wave). Una `sin` f32 son ~10-25 ns:
4000 celdas ya son 100 µs, aceptable, pero en los efectos que la piden 4 veces
por celda (pulsar) conviene una tabla de 1024 entradas + interpolación lineal:

```rust
struct Trig { sin: [f32; 1025] }   // 4 KB, sin[c] = sin(2π c/1024)
impl Trig {
    fn sin_cos(&self, turns: f32) -> (f32, f32) { /* turns mod 1, índice + lerp */ }
}
```

---

## 2. Color: de HSV a la rampa del álbum

### 2.1 El problema

cli-viz colorea con HSV y el tono es **aleatorio o por posición**:

| Efecto | Hue | Sat | Val |
|---|---|---|---|
| bars | `i/bars + hue_offset` | `0.8 + 0.2*(j/h)` | `0.7 + 0.3*(j/h)` |
| wave | `x/width + hue_offset` | `0.8 + 0.2*k` | `0.7 + 0.3*k` |
| circle | `angle/360 + hue_offset` | `0.7 + 0.3*energy` | `0.7 + 0.3*energy` |
| particles | `p.hue + hue_offset` (aleatorio) | 0.8 | `0.7 + 0.3*life` |
| flame | `0.05 + (1-heat)*0.08` | `0.8 + 0.2*heat` | `0.6 + 0.4*heat` |
| matrix | `x/width + hue_offset (+bass*0.3)` | `0.7*proximity` | `max(0.4,prox)*0.8` |
| starfield | `star.hue + hue_offset + mid*0.2` | `min(1, 0.5 + d/20 + pulse*0.3)` | `min(1, 0.5 + d/20 + pulse*0.5)` |
| fractal | `value + hue_offset + treble*0.3` | `0.7 + 0.3*min(1,v*2)` | `0.7 + 0.3*v` |
| neural | `(x/w + y/h)/2 + hue_offset + …` | `0.7 + 0.3*I` | `min(1, 0.5 + 0.5*I)` |
| neural-lite | depende de `color_mode` (3 modos) | 0.8 | `0.5…1.0` |
| pulsar | `ring.hue + hue_offset`, `color_mode` 0/1/2 | `0.5 + 0.5*b` | `0.4 + 0.6*b` |
| stick | `hue_offset + energy` | 0.8 | `0.7 + 0.3*energy` |

Y `visualizer_base.py` L69-93 convierte HSV→RGB→**cubo 6×6×6 de curses**
(216 colores indexados, `get_color_pair` L40-67). Eso es pérdida pura: en
ratatui hay `Color::Rgb(r,g,b)` y no hace falta cuantizar a 216.

### 2.2 La regla

> **La luminancia la manda la rampa del álbum; el tono solo elige el matiz.**

`Palette::ramp` son 12 colores ordenados por luminancia (`bg_deep` → `bg` →
`primary` → `secondary` → `text`). `ramp_at(t)` interpola en OKLab entre pasos.

```rust
/// Traduce un color HSV de cli-viz (h,s,v ∈ 0..1) a un RGB de la paleta.
pub fn viz_color(pal: &Palette, h: f32, s: f32, v: f32) -> Rgb {
    // 1. La rampa decide el brillo. `s` solo empuja un poco: un color muy
    //    saturado en cli-viz es un color "importante", y en la rampa eso es un
    //    paso más alto.
    let t = (0.05 + 0.90 * v * (0.80 + 0.20 * s)).clamp(0.0, 1.0);
    let base = pal.ramp_at(t);
    // 2. El tono elige hacia dónde tira ese paso, con un peso acotado (k <= 0.25)
    //    para no romper el orden de luminancia.
    let target = if h < 0.5 { pal.primary } else { pal.secondary };
    let k = 0.25 * s;
    let mixed = mix_oklab(base, target, k);
    // 3. Se re-ancla la luminancia al valor de la rampa para que la composición
    //    por capas (blend aditivo) siga siendo predecible.
    with_value(mixed, luma(base).max(0.02))
}
```

`h < 0.5` es "medio arcoíris" y `h >= 0.5` el otro. Un efecto que rota el tono
con el tiempo (matrix, pulsar en modo rainbow) acaba oscilando entre `primary`
y `secondary` a 8 s por ciclo, que es lo mismo que hacía cli-viz con
`hue_offset` pero sin salirse de la portada.

Alternativa más expresiva si el efecto está **delante** del arte (ahí se puede
gastar contraste): tres rampas virtuales y el tono elige una.

```rust
fn ramp_variant(pal: &Palette, t: f32, h: f32) -> Rgb {
    let base = pal.ramp_at(t);
    let target = match h { x if x < 1.0/3.0 => pal.primary,
                           x if x < 2.0/3.0 => pal.text,
                           _ => pal.secondary };
    let mixed = mix_oklab(base, target, 0.35);
    // el techo de luminancia no se pasa nunca del 15 % del paso original
    let cap = luma(base) * 1.15 + 0.02;
    if luma(mixed) > cap { scale(mixed, cap / luma(mixed).max(0.001)) } else { mixed }
}
```

### 2.3 Casos que no son "hue → matiz"

| Efecto | Qué hacía el HSV | Qué hace la paleta |
|---|---|---|
| **flame** | rojo→amarillo por calor | `ramp_at(0.18 + 0.78*heat)`: cuerpo oscuro, punta casi blanca. El calor ya sube por la rampa, no hace falta "fuego" naranja. Con `h < 0.5` no se toca el tono: el fuego del álbum es del color del álbum. |
| **matrix** | verde fijo | `ramp_at(0.30 + 0.65*proximity)` para la cola y `ramp_at(0.95)` para la cabeza. El "brillo de cabeza" es el paso 11-12 (casi `text`), que es lo que se lee como cabeza. |
| **neural** | hue según (x+y) | `ramp_at(0.15 + 0.8*intensity)` y el `hue` de la neurona (posición) elige `primary`/`secondary` con `ramp_variant`. |
| **pulsar** `color_mode==1` (mono) | `hue_offset` | `t` puro: todos los anillos `ramp_at(brightness)`. Es el modo "portada monocroma". |
| **pulsar** `color_mode==2` (complementario) | `+0.5` si hue>0.5 | `if hue > 0.5 { pal.secondary } else { pal.primary }` en el paso alto de la rampa. |
| **stick** | hue = energía | `ramp_at(0.55 + 0.4*energy)`: el muñeco se ilumina con el beat. |
| **bars** gradiente | `j/bar_height` | `ramp_at(0.30 + 0.65*(j/h))`: base oscura, punta brillante. |

### 2.4 Contraste y capas

- **Detrás del arte**: techo de luminancia `0.55` sobre la rampa
  (`ramp_at(t * 0.55)`), y se multiplica por `1 - 0.75*art_coverage` (§4.6).
- **Delante del arte**: `1.0`; si el arte es claro en esa celda, hay que invertir
  el paso (§4.6).
- Nunca dos pasos de rampa consecutivos en la misma celda con blend `Normal`:
  se pierde el detalle. Para eso están `Add` y `Screen`.
- `Color::Rgb` de ratatui acepta `u8`; convertir una vez por celda y reutilizar.
  Nada de `Style::fg(Color::Rgb(...))` por celda si se puede asignar
  `cell.fg = Color::Rgb(..)` directo (los campos `fg`/`bg` de `Cell` son
  públicos en ratatui 0.30).

---

## 3. Los efectos

Plantilla de cada ficha: origen y líneas, qué se ve, estado + parámetros, algoritmo
con las fórmulas exactas, uso del audio, color, coste por frame y notas de port.
El coste se da para un rect de **100×40** (4000 celdas), que es el caso típico de
la franja de visualización en un terminal de 130 columnas.

Los 12 efectos van de 3.1 a 3.12; el afinador (3.13) es un widget aparte.

---

### 3.1 `bars` — Spectrum Bars

**Origen** `visualizers/bars.py` L11-33 · **Rol** el efecto de referencia: barato,
legible, siempre correcto.

**Qué se ve** 50 barras verticales de bloques llenos con degradado de brillo hacia
la punta y boost de graves; la de la izquierda (bombo) es la más alta.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `bars` | 50 | fijo (no hay tecla) |
| `boost` | 1.5 | 0.5..5.0, paso 0.1 (`b`/`B`) |

**Algoritmo** (literal)

```
bar_width = max(1, width // 50)
n         = min(50, width // bar_width)
for i in 0..n:
    k       = min(int(i**1.3) + 1, 1023)            # índice de bin FFT
    amp     = spectrum[k] * (1 + boost * (1 - i/50))
    bh      = min(int(amp * (height-4) * 3), height-4)
    for j in 0..bh:
        hue = (i/50 + hue_offset) mod 1
        sat = 0.8 + 0.2 * (j/bh)
        val = 0.7 + 0.3 * (j/bh)
        draw(height-3-j, i*bar_width, "█" * bar_width, hsv(hue,sat,val) | BOLD)
```

Detalles que importan:

- `i**1.3`: `i=0→1`, `i=10→20` (bin 21), `i=25→66`, `i=49→158`. Con
  `bin_width=21.53 Hz`, la **última barra lee 3.4 kHz**: el efecto solo enseña
  graves y medios-bajos. Las barras de la derecha casi nunca suben con música real.
- El `*3` sobre `(height-4)` compensa que un bin típico vale ~0.02-0.05: con un
  bombo fuerte (`spectrum[2]≈40`) satura a `height-4` sin despeinarse.
- Escribe `"█"*bar_width` de una vez: `bar_width = max(1, 100//50) = 2` en un
  terminal de 100 columnas.

**Audio** Los índices `int(i^1.3)+1` cubren bins 1..158 = 21 Hz..3.4 kHz.

**Adaptación a 64 bandas** El warp `i^1.3` **sobra**: existía para descomprimir
los bins lineales, que se apiñan en los graves. Las bandas log ya son
perceptualmente lineales. Con `nbars` barras:

```rust
let k = ((i as f32 / (nbars - 1) as f32) * 63.0).round() as usize;   // 0..63
let amp = a.bands[k] * (1.0 + boost * (1.0 - i as f32 / nbars as f32));
let bh  = ((amp * (h - 4) as f32 * 3.0) as usize).min(h - 4);
```

`boost` deja de ser 1.5 y pasa a **0.6** por defecto: con bandas normalizadas el
término `(1 + boost*(1-i/n))` es una pendiente, no una amplificación; 1.5 hace
que la barra 0 sea 2.5× la última y aplaste el espectro. Calibración: con una
pista de hip-hop, la barra 0 debe tocar el techo en cada bombo y las bandas 40+
quedarse a un tercio.

**Color** `ramp_at(0.30 + 0.65*(j/bh))` — el degradado vertical ya está en el
original (`val` sube con `j`) y la rampa lo reproduce sin tocar el tono.

**Coste** `n * bh` ≤ `width*(height-4)` = **3 600 celdas** en el peor caso; típico
1 200. Ops: 50 lecturas de banda. Trivial.

**Rect pequeño**

- `h < 8` → una sola fila de bloques `▁▂▃▄▅▆▇█` con `idx = (bands[k]*7.0) as usize`
  por banda; `nbars = min(32, w)`.
- `w < 64` → `nbars = w`, `bar_width = 1`.
- `h < 5` → no dibujar (el efecto no aporta nada en 3 filas).

**Port a Rust**

- Un solo bucle; escribir celda a celda (`buf[idx].set_char('█')`), no `set_string`.
- Picos opcionales de spotatui (§6): `peak[k]` con `hold 0.35 s` y
  `gravity 2.4 alturas/s`; se dibuja con `▔` o el mismo `█` en un paso más alto de
  la rampa. 64 floats de estado, coste despreciable, y es lo que hace que el
  efecto parezca "de verdad".

---

### 3.2 `wave` — Wave

**Origen** `visualizers/wave.py` L11-36 · **Rol** el más barato de todos, y el
único que no tiene ni un byte de estado.

**Qué se ve** Una onda que ocupa el ancho entero, suma de 20 senos con fase
independiente, que se retuerce con la música.

**Estado y parámetros** Ninguno. `phase` sale de `time.time()` directamente
(L18-21), así que el efecto no se puede pausar.

**Algoritmo**

```
samples = width
wave[x] = 0 for all x
for i in 0..min(20, |spectrum|):
    freq  = (i + 1) * 2                              # ciclos por ancho: 2,4,...,40
    amp   = spectrum[i] * 10 * (height / 4)
    phase = time.time() * (i + 1) * 1.5
    for x in 0..samples:
        wave[x] += amp * sin(2*pi * freq * x / samples + phase)

for x in 0..width:
    y = int(mid_y + wave[x])
    if 0 <= y < height-1:
        k = abs(wave[x]) / (height/4)
        hue = (x/width + hue_offset) mod 1
        sat = 0.8 + 0.2*k
        val = 0.7 + 0.3*k
        draw(y, x, "•", hsv(hue,sat,val) | BOLD)
```

- `int()` trunca hacia cero: para `wave[x] = -0.9` da `y = mid_y` (no `mid_y-1`),
  lo que produce un escalón visible en el cruce por cero. En el port usar
  `wave[x].round()`.
- No hay clamp: con `amp` grande la onda se sale y desaparece de golpe (los
  puntos fuera de rango simplemente no se dibujan). Frecuente con bombo.
- 20 armónicos de 2 a 40 ciclos: el resultado es más "cuerda de ruido" que onda.
  Es lo que hace que se lea bien.

**Audio** `spectrum[0..20]` = 21 Hz..430 Hz (graves y medios-bajos), uno por
armónico.

**Adaptación a 64 bandas** `bands[0..20]` (40 Hz..520 Hz) mantiene la misma
distribución. `amp` pasa a `bands[i] * (h/4) * gain` con `gain = 1.2`, porque
cli-viz tenía un `*10` sobre valores de bin diminutos.

Corrección imprescindible: normalizar para que no se salga.

```rust
let total: f32 = (0..20).map(|i| a.bands[i]).sum();
let scale = (h as f32 / 2.0 - 1.0) / total.max(0.05);   // techo del dibujo
amp_i = a.bands[i] * scale;
```

Así un bombo aplasta la onda en vez de hacerla desaparecer, que es lo que se
espera.

**Color** `ramp_at(0.35 + 0.55*k)` con `k = |wave[x]| / (h/4)` clampeado a 1: la
cresta más alta es más brillante, el eje es oscuro. `x` no toca el tono.

**Coste** 20 × w senos = **2 000 `sin`** y **≤100 escrituras** a 100 columnas.
~40 µs. El coste está en la trigonometría, no en el dibujo.

**Rect pequeño** `h < 6` → usar solo 6 armónicos y `amp` a la mitad. `w < 40` →
6 armónicos (con 40 puntos y 40 ciclos el aliasing es total).

**Port a Rust**

- `ctx.time` en vez de `time.time()` (si no, la pausa no congela la onda).
- Acumular en un `Vec<f32>` de tamaño `rect.w` reutilizado, no en un `Vec` nuevo
  por frame (el original hace `wave = [0]*samples` cada frame).
- Precalcular `sin(2π*freq*x/samples)` en una LUT (2π y el paso son constantes
  por armónico): en la práctica son 20 tablas de `w` valores que no cambian
  nunca; solo cambia la fase. Eso baja el frame a 20 sumas por columna en vez de
  20 `sin`. Muy recomendable si se compone con 3 capas.
- Mejora barata (10 líneas): variante `scope` que dibuja `AudioFrame::waveform`
  directamente en vez de la suma de senos. Se elige con
  `Params::source: Bands | Scope`.

---

### 3.3 `circle` — Circle Spectrum

**Origen** `visualizers/circle.py` L10-35 · **Rol** relleno bonito para huecos
pequeños, o capa base de `bloom`.

**Qué se ve** 5 anillos concéntricos que laten con los graves y cambian de glifo
según la energía.

**Estado y parámetros** Ninguno. No implementa `handle_keypress` (39 líneas, sin
teclas).

**Algoritmo**

```
cx, cy    = width//2, height//2
base      = min(height, width) // 4
for idx in 0..5:
    var    = mean(spectrum[:20]) * base * 1.5     # OJO: dentro del bucle, pero constante
    r      = base - idx*3 + var
    for angle in 0..360 step 5:                    # 72 puntos
        rad = radians(angle)
        x   = int(cx + r*cos(rad));  y = int(cy + r*sin(rad))
        if 0 <= y < height-1 and 0 <= x < width:
            ch  = "•" if energy < 0.1 else ("*" if energy < 0.2 else "★")
            hue = (angle/360 + hue_offset) mod 1
            sat = 0.7 + 0.3*energy;  val = 0.7 + 0.3*energy
            draw(y, x, ch, hsv(...) | BOLD)
```

- `mean(spectrum[:20])` se recalcula 5 veces pero da lo mismo: **los 5 anillos se
  mueven como un bloque**, separados 3 celdas. Es el defecto obvio del efecto.
- `y < height-1`: se descarta la última fila (mitigación del error de curses al
  escribir la esquina inferior derecha).
- `"★"` es East_Asian_Width = **Ambiguous**: en terminales con fuente CJK ocupa 2
  celdas y rompe el grid. Ver Anexo A.

**Audio** `spectrum[:20]` = 21..430 Hz, reducido a **un escalar** (la media). El
efecto solo tiene un grado de libertad: el radio. La `energy` decide el glifo y
la saturación.

**Adaptación a 64 bandas** `mean(bands[0..24])` (40..520 Hz). Y romper la
uniformidad, que es lo que arregla el efecto:

```rust
let var_i = mean(&a, i*4, i*4 + 4) * base * 0.9;   // 5 tramos distintos
let r = base - i as f32 * 3.0 + var_i;
```

**Color** `ramp_at(0.55 + 0.4*energy)` para todo el anillo, con `ramp_at` por
ángulo si se quiere el degradado cromático: `ramp_at(0.4 + 0.3*(angle/360))`.
Los tres glifos pasan a mapear el paso: `0.6` → `·`, `0.8` → `*`, `1.0` → `●`.

**Coste** 5 × 72 = **360 candidatos**, ≤360 escrituras. Trivial (el más barato
junto con wave).

**Rect pequeño** `min(h,w) < 12` → 1 anillo de radio `min(h,w)/2 - 1` y 16 puntos
(cada 22.5°); sigue leyéndose.

**Port a Rust**

- **Corregir el aspecto**: `x = cx + r*cos(θ)*CELL_ASPECT`. Sin esto sale una
  elipse achatada y no un círculo. Es la diferencia entre "un anillo" y "un
  borrón".
- Grosor: si `energy > 0.5`, pintar también `x±1`; un anillo de 1 celda se ve
  pobre a 100 columnas.
- `0 <= y < height` (no `h-1`): en ratatui no hay excepción en la esquina.
- 72 senos/cosenos por anillo → 360 pares: usar la LUT de §1.5.

---

### 3.4 `particles` — Particles

**Origen** `visualizers/particles.py` L12-57 · **Rol** la capa "ambiente" que
mejor combina con cualquier otra (es la que hace que `aurora` funcione).

**Qué se ve** Una fuente de partículas en la mitad inferior que suben y caen con
gravedad mientras se apagan.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `max_particles` | 100 | 50..500, paso 50 (`p`/`P`) |
| `particles` | vacío | `Vec<Particle>` |

`Particle { x, y, vx, vy, life, hue, size }` — `size` es el **glifo**, no el
tamaño: `{'.', '*', '+', '•', '○', '◌', '◦'}`.

**Algoritmo**

```
# spawn
if |particles| < max and energy > 0.1:
    n = int(energy * 10)
    repeat n:
        x    = width//2 + randint(-10, 10)
        y    = height - 5
        vx   = uniform(-2, 2)
        vy   = uniform(-5, -2)
        life = uniform(0.5, 1.0)
        hue  = random()
        size = choice(glifos)

# update + draw
for p in particles:
    p.x  += p.vx
    p.y  += p.vy
    p.vy += 0.1             # gravedad
    p.life -= 0.02
    if p.life > 0 and 0 <= p.y < height-1 and 0 <= p.x < width:
        val = 0.7 + 0.3*p.life
        draw(int(p.y), int(p.x), p.size, hsv(p.hue + hue_offset, 0.8, val) | BOLD)
```

**Semántica real de las constantes** (y por qué hay que rescalarlas):

- `vy0 ∈ [-5,-2]` **celdas por frame** = 120..300 celdas/s a 60 fps. La partícula
  cruza 40 filas en ~0.15 s.
- `vy += 0.1` por frame = **360 celdas/s²**. Es una explosión, no gravedad.
- `life ∈ [0.5,1.0]` con `-= 0.02` = 25..50 frames = **0.42..0.83 s**.
- Resultado: es una fuente de chispas que sube y cae en medio segundo. Bonito,
  pero no es un sistema de partículas "físico".

**Audio** Solo `energy` como puerta y como cantidad (`energy > 0.1`,
`n = int(energy*10)`). Las bandas no se usan.

**Adaptación a 64 bandas** El beat debe dominar el estallido:

```rust
let n = ((a.beat_pulse * 8.0 + a.energy * 4.0) as usize).min(12);
if a.energy > 0.04 { spawn(n) }
```

Y para que la fuente responda al espectro, elegir `vx` con las bandas agudas y
`vy` con las graves: `vx = U(-2,2) * (20.0 + 40.0*a.bands[50])`,
`vy = -(2.0 + 4.0*a.bass) * 60.0`. Opcional; el original es más simple.

**Color** `ramp_at(0.5 + 0.45*life)`: la partícula nace brillante y muere oscura
(el original hacía lo mismo con `val = 0.7+0.3*life`). El `hue` aleatorio por
partícula se sustituye por un `step` aleatorio en `{0, 1}` que elige
`primary`/`secondary` en el paso alto de la rampa.

**Coste** ≤`max_particles` escrituras (100 típico, 500 máximo) + ≤10 spawns.
Trivial.

**Rect pequeño** `max = min(max_particles, rect.area()/12)`; si `h < 10`, spawn en
`h-2` en vez de `h-5`.

**Port a Rust**

- Simulación en **unidades por segundo** (regla 1.4): multiplicar `vx`, `vy` y la
  gravedad por 60, y avanzar con `dt`.
- Reutilizar el `Vec` con `swap_remove` al morir una partícula (el original
  reconstruye la lista entera cada frame).
- `size: u8` índice a un LUT de glifos `&'static [char; N]`, no `&str`.
- Si el efecto va **detrás** del arte, bajar el techo de la rampa a 0.6 y no
  usar `BOLD` (el bold sobre arte dither se come el contraste).

---

### 3.5 `flame` — Flame

**Origen** `visualizers/flame.py` L20-131 · **Rol** el efecto con más "cuerpo" por
línea de código: un autómata de calor de una dimensión con difusión.

**Qué se ve** Una llama de 80×30 que sube desde la base, se abre en el centro,
parpadea y se enciende con el bombo, con un tronco y brasas en la fila inferior.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `flame_width` | 80 | 20..200, paso 5 (`w`/`W`) |
| `flame_height` | 30 | 10..50, paso 2 (`h`/`H`) |
| `flame_grid` | 30×80 f32 | buffer de calor 0..1 |
| `flame_cooling` | 30×80 f32 | buffer de enfriamiento |

Dimensiones efectivas por frame: `fh = min(h-2, flame_height)`,
`fw = min(w, flame_width)`; si cambian, se **realloca a cero** (el efecto se
reinicia al redimensionar).

**Algoritmo**

```python
# 1. Enfriamiento (L31-42). OJO: 2400 llamadas a random() por frame.
for i in 0..fw:
    edge = 0.2 * (1 - min(i, fw-i-1) / (fw/2))          # 0 en el centro, 0.2 en los bordes
    for j in 0..fh:
        height_cooling = 0.1 * (j / fh)                 # crece hacia ABAJO (base de la llama)
        cooling[j][i]   = random() * 0.2 + edge + height_cooling

# 2. Aplicar
grid = clip(grid - cooling, 0, 1)

# 3. Audio
bass = mean(spectrum[:10]) * 3
mids = mean(spectrum[10:30]) * 2

# 4. Foco de calor en la fila inferior (L45-64)
for i in 0..fw:
    center_bias = 1 - 0.5 * abs(i - fw/2) / (fw/2)      # 1 en el centro, 0.5 en los bordes
    fluct       = random() * 0.3 + 0.7
    heat        = bass * center_bias * fluct
    if random() < 0.1 * (bass + mids):                   # chisporroteo
        heat += random() * 0.5
    grid[fh-1][i] = min(1, heat)                         # ASIGNACIÓN, no suma

# 5. Propagación hacia arriba (L67-88), in-place
for y in 1..fh:
    ty = fh - y - 1
    sy = ty + 1
    for x in 0..fw:
        l = max(0, x-1); r = min(fw-1, x+1)
        new = (0.2*grid[sy][l] + 0.6*grid[sy][x] + 0.2*grid[sy][r]) * (0.95 + 0.05*bass)
        grid[ty][x] = min(1, new)

# 6. Render (L90-113)
chars = ' .,:;=+*#%@'                                    # 12, los dos primeros son espacio
screen_y = height - fh + y - 1
screen_x = (width - fw)//2 + x
if heat > 0.01:
    ch  = chars[min(11, int(heat*12))]
    h   = 0.05 + (1-heat)*0.08                           # 0.13 (rojo) → 0.05 (naranja)
    s   = 0.8 + heat*0.2
    v   = 0.6 + heat*0.4
    draw(screen_y, screen_x, ch, hsv(h,s,v))

# 7. Tronco (L117-131), fila height-1
log_width = fw//2, centrado: '▄' con RGB fijo (139,69,19)
+ 5 brasas por frame en posiciones aleatorias, hsv(0.05, 1.0, 0.8)
```

Las tres claves del efecto, por si hay que reimplementarlo sin el original:

1. **La propagación es in-place y en orden ascendente**. `ty` baja mientras `y`
   sube, y `sy = ty+1` ya fue escrito este mismo frame. Eso da el arrastre
   vertical: el calor de la base "empuja" la columna. Si se hace con doble
   buffer, la llama se queda corta y plana.
2. **El enfriamiento es mayor en la base que en la punta** (`height_cooling`
   crece con `j`, y `j` grande = base). Contraintuitivo pero es lo que evita que
   la llama sea una columna sólida.
3. El `0.2/0.6/0.2` es un kernel gaussiano de 3: da la silueta redondeada.

**Audio** `bass = mean(spectrum[:10])*3` (22..194 Hz) y `mids = mean[10:30]*2`
(215..624 Hz). **Los dos son graves**: la llama solo responde a bombo y bajo.

**Adaptación a 64 bandas** `bass = mean(bands[0..18])`, `lowmid = mean(bands[18..30])`,
ambos ya 0..1 → quitar los `*3`/`*2` y poner `gain = 1.4`. El chisporroteo:
`0.1*(bass+lowmid)` puede pasar de 1 con bandas normalizadas → `clamp(0, 1)`.

**Color** `ramp_at(0.18 + 0.78*heat)`: el cuerpo es casi `bg` y la punta sube a
`secondary`/`text`. El glifo ya da la densidad; la rampa da el brillo. El tronco:
`mix(pal.bg, pal.primary, 0.30)`; las brasas: `ramp_at(0.85)`.

**Coste** `fw*fh` (enfriamiento) + `fw*(fh-1)` (propagación) + `fw*fh` (render)
≈ **7 100 operaciones** con 2 400 escrituras de celda a 80×30. El coste real no
son las operaciones, son las **2 400 llamadas a `random()` por frame** (en Python
son ~3 ms).

**Rect pequeño** `fh < 8 || fw < 16` → no dibujar. Con `w < 60`, `fw = w` y
`fh = h` directamente (los 80×30 fijos no tienen sentido en un rect pequeño) y
bajar el enfriamiento a `0.15`.

**Port a Rust**

- **Un solo buffer**: `cooling` no necesita ser un array. Se resta inline y se
  usa un hash de `(x, y, frame)` en vez de `random()`:
  ```rust
  #[inline]
  fn hash01(x: u32, y: u32, f: u64) -> f32 {
      let mut h = f ^ (x as u64).wrapping_mul(0x9E37_79B9) ^ (y as u64).wrapping_mul(0x85EB_CA6B);
      h ^= h >> 33; h = h.wrapping_mul(0xFF51_AFD7); h ^= h >> 29;
      (h & 0xFFFF) as f32 / 65535.0
  }
  ```
  2 400 hashes = ~10 µs. Sin estado por celda y determinista.
- El buffer es `Vec<f32>` de `fw*fh` con índice `y*fw + x`, dimensionado en
  `resize`; `fill(0.0)` solo al cambiar de tamaño (no cada frame: la llama tiene
  memoria).
- El tronco y las brasas: fuera por defecto. Si el efecto va delante del arte,
  la fila `h-1` ya la ocupa el transporte.
- `'▄'` es block element (Ambiguous, ancho 1 en la práctica). Aceptable, pero es
  el único glifo del efecto que no es ASCII.

---

### 3.6 `matrix` — Matrix Rain

**Origen** `visualizers/matrix_rain.py` L50-153, spawn en L27-48 · **Rol** la capa
"textura" ideal para `Multiply` (ver §4.7, preset `void`).

**Qué se ve** Columnas de glifos que caen, con cola que se apaga, cabeza brillante,
velocidad ligada a la banda que le tocó en suerte, y un flash en las esquinas en
cada beat.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `density` | 0.2 | 0.05..1.0, paso 0.05 (`d`/`D`) |
| `speed_factor` | 1.0 | 0.1..3.0, paso 0.1 (`s`/`S`) |
| `length_factor` | 1.0 | 0.1..3.0, paso 0.1 (`l`/`L`) |
| `spawn_threshold` | 0.05 | fijo |
| `beat_cooldown` | 0 | 5 frames tras cada beat |
| `drops` | `[]` | lista de `{x, y, speed, length, char_idx, chars, bright, freq_index}` |

`chars` = ASCII 33..126 (94) + **U+30A0..U+30FE** (95 katakana *fullwidth*,
ancho 2 → **ilegal en el port**, ver Anexo A).

**Algoritmo**

```python
# Beat (L52-58)
if energy > 0.2 and beat_cooldown == 0:  beat_active = True;  beat_cooldown = 5
elif beat_cooldown > 0:                  beat_cooldown -= 1

bass = mean(spectrum[:10]) * 2
mids = mean(spectrum[10:30])
treble = mean(spectrum[30:])

# ¿Cuántos drops intentar este frame? (L66-68)
base_spawn_rate = int(width * 0.15 * density)
spawn_count     = base_spawn_rate + int(energy * 15) + (5 if beat else 0)

if energy > spawn_threshold:
    spawn_drops(spawn_count, width, spectrum)

# spawn_drops: cada intento puede fallar (probabilidad = amplitud×3)
for _ in range(spawn_count):
    pos  = randint(0, len(spectrum)//2)
    w    = spectrum[pos] * 3.0
    if random() < w:
        drop = { x: randint(0, width-1), y: 0,
                 speed: uniform(0.2, 0.8) * speed_factor,
                 length: randint(3, 20) * length_factor,
                 chars: "".join(choice(chars) for _ in range(randint(5,15))),
                 char_idx: 0, bright: random() < 0.2,
                 freq_index: pos }

# update + draw (L75-126)
for drop in drops:
    freq_amp = spectrum[drop.freq_index] * 3.0
    drop.y  += drop.speed * (1 + freq_amp)
    if random() < 0.1:
        drop.char_idx = (drop.char_idx + 1) % len(drop.chars)
    if drop.y >= height: drop

    trail_start = int(max(0, drop.y - drop.length))
    trail_end   = int(drop.y)
    for y in trail_start..=trail_end:
        proximity = 1 - (trail_end - y) / drop.length
        ch = drop.chars[(drop.char_idx + y % len(drop.chars)) % len(drop.chars)]
        if y == trail_end:                       # cabeza
            hue = (drop.x/width + hue_offset + bass*0.3) mod 1
            sat = 0.7 if drop.bright else 0.5
            val = 1.0 if drop.bright else 0.7
            bold = True
        else:                                    # cola
            hue = (drop.x/width + hue_offset) mod 1
            sat = 0.7 * proximity
            val = max(0.4, proximity) * 0.8
            bold = False
        if beat_active and drop.bright:
            sat = min(1.0, sat + 0.3); val = min(1.0, val + 0.3)
        draw(y, drop.x, ch, hsv(hue,sat,val) | bold)
```

Extras: barra de debug en la última fila (L136-138), **flash de 4 esquinas con
`✧` en el frame del beat** (L141-147), y `beat_active` que se apaga al siguiente
frame (L150-152) — o sea, el "flash" dura exactamente 1 frame.

**Audio** La banda que le toca a cada drop (`freq_index`) decide su velocidad:
`y += speed*(1+3*amp)`. Con `amp` típico 0.02, el multiplicador es 1.06 (nada);
con un pico de 40, es 121 (teletransporte). Es un efecto con una no-linealidad
brutal: **en el port hay que clampear** `1 + freq_amp` a `[1, 3]`.

**Adaptación a 64 bandas** `pos = random_range(0..40)`, `w = bands[pos] * 1.2`
(con `*3` casi todos los intentos fallan con bandas normalizadas). El
`spawn_count`: `int(w*0.15*density) + (energy*8) as usize + 4*beat as usize`.

**Color** La firma verde se pierde, pero la estructura se mantiene:

- cola: `ramp_at(0.30 + 0.65*proximity)`
- cabeza: `ramp_at(0.95)` (casi `text`: es lo que se lee como cabeza)
- `bright`: `ramp_at(t + 0.12)` clampeado
- beat: `t + 0.10` y `+BOLD`
- esquinas en beat: `ramp_at(1.0)`

**Coste** Σ longitudes de cola, **sin cota**. Con `density=1.0`, `w=200`:
`base_spawn_rate=30`, `+energy*15`, `+5` = ~50 intentos/frame con probabilidad
~0.9 → **45 drops nuevos por frame**; a 40 frames de vida y 20 celdas de cola:
~1 800 drops vivos × 20 = **36 000 escrituras/frame**. Es el agujero de
rendimiento más grande del repo.

**Rect pequeño** `area < 400` → `max_drops = 32` y `length = min(length, h/2)`.
Si `w < 30`, `max_drops = 12`.

**Port a Rust**

- **Cap duro**: `max_drops = (rect.area()/10).clamp(16, 240)`; al llegar, no se
  hacen spawns (el original nunca limita y confía en que el usuario no suba
  `density`).
- **Sin `String` por drop**: los 5..15 `chars` aleatorios de cada drop existen
  solo para que el glifo cambie con el tiempo. En el port:
  `let ch = GLYPHS[hash(x, y, frame/4) as usize & 0xff]` con un LUT de 256
  glifos de **ancho 1**. Ahorra 1 alloc por drop y 20 lookups por frame.
- `beat_cooldown` en frames (5) → en segundos: `0.083`, con `-= dt`.
- `beat_active` de 1 frame → se convierte en `beat` directo de `AudioFrame`.
- Estado: `Vec<Drop>` con `Drop { x: u16, y: f32, speed: f32, len: f32, freq: u8, bright: bool }`
  = 20 bytes. 240 drops = 5 KB, cabe en L1.
- Si va detrás del arte: cap de rampa 0.55 y sin BOLD.

---

### 3.7 `starfield` — Starfield Warp

**Origen** `visualizers/starfield_warp.py` L31-258 · **Rol** el mejor "fondo" de
todos: llena el rect sin tapar nada y reacciona al beat de forma inequívoca.

**Qué se ve** Estrellas que salen del centro y aceleran hacia fuera; en cada beat
un anillo de puntos se expande y nacen ondas sinusoidales perpendiculares a un
ángulo aleatorio.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `star_count` | 100 | fijo (multiplicado por `star_density`) |
| `warp_speed` | 0.5 | 0.1..2.0, paso 0.1 (`w`/`W`) |
| `waveform_intensity` | 1.0 | 0.1..2.0, paso 0.1 (`i`/`I`) |
| `waveform_count` | 6 | fijo |
| `star_density` | 1.0 | 0.2..3.0, paso 0.2 (`n`/`N`) |
| `beat_history` | `[0.0]*10` | ventana para el detector propio |
| `phase`, `beat_pulse`, `bass_level`, `mid_level`, `treble_level` | — | — |

`Star { x, y, angle, speed, size, hue, birth_time }`,
`Wave { angle, life, freq, amplitude, hue, width }`.

**Algoritmo**

```python
# Bandas (L36-40): ojo, "treble" llega solo a 624 Hz
bass   = mean(spectrum[:10]) * 2
mid    = mean(spectrum[10:20])
treble = mean(spectrum[20:30])

# Beat propio: energía contra su media de 10 frames (L42-53)
beat_history.append(energy); beat_history.pop(0)
avg     = mean(beat_history)
is_beat = energy > 1.5*avg and energy > 0.4
if is_beat: beat_pulse = min(1.0, beat_pulse + 0.7*bass)
else:       beat_pulse *= 0.8

phase += (0.5 + energy) * warp_speed                 # radianes por FRAME

# Spawn de estrellas: como mucho UNA por frame (L57-82)
if len(stars) < 100*star_density and random() < 0.3 + bass*2:
    angle = uniform(0, 2pi); dist = uniform(1, 3)
    size  = '*' if bass>0.5 else '+' if mid>0.3 else '·' if treble>0.2 else '.'
    stars.append({ x: cx + dist*cos(angle), y: cy + dist*sin(angle), angle,
                   speed: 0.1 + uniform(0,0.4)*(1+bass), size,
                   hue: random(), birth_time: phase })

# Anillo de beat (L84-99)
if beat_pulse > 0.1:
    radius = int(max(width,height) * 0.4 * beat_pulse)
    for angle in 0..360 step 15:                     # 24 puntos
        ch = '°' if angle % 30 == 0 else '·'

# Update de estrellas (L102-156)
for s in stars:
    wobble   = treble * sin(phase*5 + s.birth_time)
    ang      = s.angle + wobble*0.2
    s.speed += 0.02*bass + 0.01*energy
    s.x     += cos(ang) * s.speed * (1+energy)
    s.y     += sin(ang) * s.speed * (1+energy)
    dist     = hypot(s.x-cx, s.y-cy)
    if on-screen and dist < max(w,h):
        hue = (s.hue + hue_offset + mid*0.2) mod 1
        sat = min(1, 0.5 + dist/20 + beat_pulse*0.3)
        val = min(1, 0.5 + dist/20 + beat_pulse*0.5)      # más brillo cuanto más lejos
        ch  = s.size
        if s.speed > 0.5 + bass:  ch = '-' if |cos(ang)|>|sin(ang)| else '|'
        if s.speed > 1.0 + bass/2: ch = '=' if |cos(ang)|>|sin(ang)| else '‖'
        if is_beat and random() < bass: attr |= REVERSE
```

Ondas (L158-250): nacen en el beat con `bass > 0.3`, `n = 1 + int(bass*3)` a la
vez, con `angle` aleatorio, `life = 1.0`, `freq = 3 + norm_freq*10` (donde
`norm_freq` = índice de la banda dominante / 30), `amplitude = 5 + bass*15`,
`hue = (offset + norm_freq)`, `width = 1 + int(energy*2)`. Cada frame:

```python
wf.life -= 0.03 + 0.02*treble
max_dist  = min(width, height)//2 * wf.life
for dist in 1..max_dist step 2:
    off  = wf.amplitude * sin(dist / wf.freq * pi) * wf.life
    perp = wf.angle + pi/2
    x = cx + dist*cos(wf.angle) + off*cos(perp)
    y = cy + dist*sin(wf.angle) + off*sin(perp)
    if dist % 3 == 0: ch = '*' if (intensity > 0.7 or beat_pulse > 0.5) else '+'
    else:             ch = '·' if intensity > 0.5 else '.'
    intensity = wf.life * (1 + bass*0.5)
```

**Audio** `bass` 22..194 Hz, `mid` 215..409, `treble` 215..624 (¡solapados!). El
wobble de `treble` y la selección de glifo por nivel se pisan, pero
funcionalmente da igual: es decoración.

**Adaptación a 64 bandas** `bass = mean(0,18)`, `mid = mean(18,24)`,
`treble = mean(24,32)` para respetar la intención original (todo por debajo de
800 Hz). Para que el `wobble` venga de verdad de los agudos, usar `mean(48,64)`
con ganancia 0.5 — es una mejora, no una fidelidad.

**Color** `ramp_at(0.45 + 0.5*dist/max_dim + 0.35*beat_pulse)` clampeado: las
estrellas lejanas son brillantes (como el original), y el beat sube el paso.
El anillo de beat, `ramp_at(1.0)` sin BOLD para que no compita.

**Coste** ≤300 estrellas (con `star_density=3.0`; el spawn es 1/frame, así que en
la práctica 60-150) + ~10 ondas × ≤(min(w,h)/4) puntos ≈ **550 escrituras**.
Barato. La trigonometría: 2 `sin` + 2 `cos` por estrella por frame + 1 `sin` por
punto de onda ≈ 1 000 trig/frame → 20-30 µs.

**Rect pequeño** `area < 600` → `star_count = 40`, sin ondas (`waveform_count = 0`)
y sin anillo de beat.

**Port a Rust**

- **`'‖'` (U+2016) fuera**: es Ambiguous y en muchas fuentes mide 2 celdas. Usar
  `'|'` o `'‖'→'H'`. `'°'` (U+00B0) es Ambiguous también: en el puerto usar
  `'o'`/`'*'`.
- `beat_history` + detector propio: usar `AudioFrame::beat` y
  `AudioFrame::beat_pulse` (ya existen). Reescribir el pulso como
  `pulse = max(pulse, 0.7 + 0.3*bass)` en beat y `pulse *= 0.82^(dt*60)` si no.
- `waveform_life` Vec → `[Wave; 12]` + `len: usize` (nunca hay más de
  `6 + int(bass*3)` = 9).
- `phase += ... * dt * 60.0`: en el original es por frame; con `warp_speed=0.5` y
  `energy=0.2` son 0.35 rad/frame = 21 rad/s = 3.3 vueltas/s. Es rapidísimo.
  Para spote, `* 0.5`.
- Estrellas en `Vec<Star>` con capacidad `3*star_count`; `swap_remove` al salir.

---

### 3.8 `fractal` — Fractal Universe

**Origen** `visualizers/fractal_universe.py`, `compute_fractal` L46-171 y
`draw` L173-283 · **Rol** el efecto "de culto": el más caro y el que más se
recuerda.

**Qué se ve** Mandelbrot (o Julia) en ASCII, con zoom que respira con los graves,
rotación continua, y en cada beat fuerte un "warp" espiral que a veces cambia de
modo y de estilo de render.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `max_iterations` | 30 | fijo |
| `zoom` / `zoom_target` | 2.5 | 0.01..10 (`+`/`-` ÷×1.2) |
| `center_x`, `center_y` | 0, 0 | ±0.1·zoom por flecha |
| `julia_mode` | false | toggle (`j`) + automático al final del warp |
| `zoom_speed` | 0.05 | fijo (interpolación por frame) |
| `julia_seed` | `-0.8 + 0.156i` | complejo animado |
| `julia_seed_history` | 20 elementos | cola |
| `beat_intensity` | 0 | 0..1, `*= 0.9` por frame |
| `rotation` | 0 | acumula `treble*0.1` por frame |
| `pattern_complexity` | 1.0 | `1 + mid*3` |
| `warp_active`, `warp_progress` | false, 0 | progreso `+= 0.05`, 20 frames |
| `render_mode` | 0 | 0 clásico, 1 gradiente, 2 psicodélico (`1`/`2`/`3`) |
| `density_chars` | 70 glifos | ver abajo |

`density_chars` (L37), 70 caracteres, índice 0 = espacio:

```
 .'`^",:;Il!i><~+_-?][}{1)(|/\tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$
```

**Algoritmo**

```python
# --- parámetros del frame (L50-99) ---
mid   = mean(spectrum[10:30])
pattern_complexity = 1 + mid*3
max_iter = int(30 * pattern_complexity)              # 30..~120
aspect   = height / width
bass     = mean(spectrum[:10]) * 2
zoom_target = clamp(2.5 - bass*5.0, 0.5, 10.0)
zoom       += (zoom_target - zoom) * 0.05            # one-pole
treble      = mean(spectrum[30:])
rotation   += treble * 0.1

# --- beats (L67-99) ---
if energy > 0.2 and now - last_beat > 0.2:
    beat_intensity = min(1, energy*2)
    if energy > 0.4 and now - last_big_beat > 1.5:
        warp_active = True; warp_progress = 0
beat_intensity *= 0.9
if warp_active:
    warp_progress += 0.05
    if warp_progress >= 1.0:
        warp_active = False
        if big_beat or random() < 0.3: julia_mode = not julia_mode
        if big_beat and random() < 0.4: render_mode = (render_mode+1) % 3

# --- semilla de Julia (L101-119) ---
if julia_mode:
    angle  = now * (0.2 + bass*0.5)
    radius = 0.7 + 0.2*sin(now*0.25) + bass*0.3
    target = radius * (cos(angle) + i*sin(angle))
    julia_seed += (target - julia_seed) * (0.1 + bass*0.3)
    julia_seed_history.append(julia_seed); if len > 20: pop(0)

# --- por celda (L122-169) ---
real = (x - width/2)  * (4.0/width) * zoom + center_x
imag = (y - height/2) * (4.0/width) * aspect * zoom + center_y
c = complex(real, imag)
if rotation != 0: c *= exp(i*rotation)
if warp_active:  c *= exp(i * warp_progress * 2pi * beat_intensity)
if julia_mode: z = c; c = julia_seed
else:          z = 0
escape = 4.0 + beat_intensity*12.0                   # 4..16
iter = 0
while abs(z) < escape and iter < max_iter:
    if   render_mode == 1: z = z*z + c + 0.01i*sin(now)     # "gradiente"
    elif render_mode == 2: z = z*z*z / (z*z + c) + c        # "psicodélico"
    else:                  z = z*z + c
    iter += 1
if iter < max_iter:
    smooth = iter + 1 - log(log(abs(z))) / log(2)
    buffer[y][x] = smooth / max_iter
else:
    buffer[y][x] = 0                                  # dentro del conjunto
```

Dibujo (L189-234): `value == 0` → Julia: relleno con hue posicional y `'●'`;
Mandelbrot: `' '`. Si no, `hue = value + offset + treble*0.3`,
`sat = 0.7+0.3*min(1,value*2)`, `val = 0.7+0.3*value` (+`beat_intensity*0.3`),
`ch = density_chars[int(value*70)]`, `BOLD` si `value > 0.7`. Y por encima: título
(L185-187), ayuda (L261-262), barra de beat (L265-269) y esquinas `╔╗╚╝` durante
el warp (L272-283) — todo eso se descarta.

**Audio** `bass` 22..194 Hz → zoom; `mid` 215..624 → iteraciones; `treble` 646 Hz+
→ rotación. La rotación es acumulativa: con `treble = 0.05` constante gira
0.05 rad/frame = 3 rad/s. Un tema con muchos agudos gira muy rápido.

**Adaptación a 64 bandas** `bass = mean(0,18)`, `mid = mean(18,30)`,
`treble = mean(30,64)`. Quitar los `*2`, `*5` y `*3`: con bandas normalizadas,
`zoom_target = 2.5 - bass*5` da valores negativos en cuanto `bass > 0.5`; usar
`zoom_target = clamp(2.5 - bass*2.2, 0.6, 6.0)` y `rotation += treble*0.02`.
Para las iteraciones, `pattern_complexity = 1 + mid*3` es correcto tal cual (0..1
de banda → 1..4× iteraciones), pero multiplicado por 30 da hasta 120:
**clampear a 60** o el frame se va a 30 ms.

**Color** `ramp_at(0.05 + 0.90*value)`. La rampa recorre `bg_deep → primary →
secondary → text`, que es exactamente lo que hacía el hue cíclico del original,
pero con la luminancia ordenada (los anillos exteriores, donde `value` es
pequeño, quedan oscuros; el borde del conjunto, brillante). El interior de
Mandelbrot: `None` (no pintar celda) si la capa va detrás del arte; si va
delante, `pal.bg_deep`.

**Coste** `w*h*max_iter` iteraciones complejas: 100×40×30 = **120 000**, ~6 flops
cada una ≈ 1 MFLOP/frame → 1-2 ms en Rust. Con `pattern_complexity` a 4 (120
iter) y 200×60: 12 000×120 = **1.44 M iteraciones** → 15-25 ms. **Rompe los
16.6 ms.** El límite real del efecto es este.

**Rect pequeño** Regla de presupuesto por área:

```rust
let area = (rect.w as f32) * (rect.h as f32);
max_iter = (30.0 * 4000.0 / area).clamp(8.0, 60.0) as u32;
// área 2000 → 60; 4000 → 30; 8000 → 15
```

Y si aún no llega: `step = 2` (calcular 1 de cada 4 celdas y replicar la
vecina), como `fractal_universe_lite.py` L93-132.

**Port a Rust**

- **Una sola pasada**: el original crea un `np.zeros((h,w))`, lo rellena y luego
  lo recorre otra vez para dibujar. Son dos pasadas y una alloc. En el puerto se
  calcula la celda y se escribe en el mismo bucle.
- **Hoist fuera del bucle**: `sin(now)` (modo 1), `cos/sin(rotation)`,
  `cos/sin(warp)` (el warp es una constante por frame), `4.0/width`, `aspect`,
  `escape²`.
- **Sin `sqrt`**: comparar `re*re + im*im < escape²`. Solo se necesita `|z|` al
  final, y solo si escapó.
- **NaN en el suavizado**: `log(log(|z|))` es NaN cuando `|z| <= 1` (pasa cuando
  el punto escapa con `|z|` pequeño en las primeras iteraciones). El original
  escupe warnings de numpy y escribe NaN, que luego se compara con `== 0` y falla.
  Port:
  ```rust
  let r2 = re*re + im*im;
  let value = if r2 > 1.0001 {
      (iter as f32 + 1.0 - (0.5 * r2.ln()).ln() / std::f32::consts::LN_2) / max_iter as f32
  } else {
      iter as f32 / max_iter as f32
  };
  ```
- **Corrección de aspecto**: `imag = (y - h/2) * (4.0/w) * CELL_ASPECT * zoom`. Con
  `aspect = h/w` el fractal sale aplastado verticalmente porque el original asume
  píxeles cuadrados.
- **Cache**: si `zoom`, `rotation`, `seed`, `center` y `max_iter` no han cambiado
  más de un epsilon, reutilizar el buffer del frame anterior. `fractal_lite` lo
  hace a lo bruto (`frame_counter % (skip+1)`); en el puerto es mejor por epsilon,
  porque el zoom se mueve siempre un poco.
- `mode 2` (`z³/(z²+c)+c`) explota numéricamente si `z²+c ≈ 0`. Si se porta,
  añadir `if (z2.norm_sqr() < 1e-6) { break; }`.
- El estado `julia_seed_history` solo se usa para dibujar la estela de la semilla
  (L236-258). Si el efecto va detrás del arte, se puede quitar entero (20
  complejos y 20 escrituras).

---

### 3.9 `neural` — Neural Dreamscape

**Origen** `visualizers/neural_dreamscape.py`, `initialize_fields` L34-56,
`seed_initial_activity` L58-95, `update_neural_field` L97-318, `draw` L319-509 ·
**Rol** el efecto "orgánico": tres campos acoplados (energía, onda, neuronas).

**Qué se ve** Una red de neuronas que pulsan, difunden energía por el campo, se
conectan con sinapsis que brillan, y una onda que se propaga por toda la pantalla
sembrada por los beats.

**Estado y parámetros**

| Campo | Default | Notas |
|---|---|---|
| `energy_field` | (h-2)×w f32 | difusión |
| `wave_field` | (h-2)×w f32 | ecuación de onda, -1..1 |
| `field` | (h-2)×w f32 | **se aloca y no se usa nunca** → fuera |
| `active_neurons` | dict `(y,x) → {strength, hue, pulse_rate, age}` | |
| `synapses` | `[{start, end, strength, active}]` | |
| `thought_particles` | `[{x, y, vx, vy, life, hue, size}]` | |
| `resonance`, `consciousness_level`, `dream_intensity` | 0 | one-pole 0.9/0.1 |
| `time_counter` | 0 | **frames**, no segundos |
| `symbols` | 19 glifos | `·•○◙★✧♦❂☼❈♠♣⚡✿❀⚘❄✺⚕` |
| `evolution_speed` | 0 | se calcula y no se usa |

**Algoritmo**

```python
# --- semilla (solo al cambiar de tamaño) ---
n = int(min(w,h) * 0.2)
for i in 0..n:                                    # círculo de radio min/4
    x = cx + r*cos(2pi*i/n); y = cy + r*sin(2pi*i/n)
    neuronas[(y,x)] = { strength: U(0.5,1), hue: U(0,1), pulse_rate: U(0.05,0.2), age: 0 }
    energy_field[y][x] = U(0.5, 1)
for _ in range(3*n):                              # sinapsis aleatorias
    conectar dos neuronas distintas al azar, strength U(0.3,1), active 0

# --- cada frame ---
time_counter += 1
bass = mean(spectrum[:10])*2; mids = mean(spectrum[10:30]); treble = mean(spectrum[30:])
resonance    = 0.9*resonance    + 0.1*(mids + treble*0.5)
consciousness= 0.95*conscious.  + 0.05*(treble / (bass + 0.01))
dream        = 0.9*dream        + 0.1*energy

if energy > 0.25 and time_counter - last_beat > 5:      # 5 FRAMES
    for _ in range(int(energy*10)):                     # partículas de pensamiento
        pos aleatoria; angle = atan2(y-cy, x-cx)
        thoughts.append({ vx: cos(angle)*(0.5+energy*2), vy: sin(angle)*(0.5+energy*2),
                          life: randint(10,30), size: U(0.5,1.5) })
        wave_field[y][x] = 1.0

if random() < 0.1*energy:                               # nacimientos
    for _ in range(int(energy*5)):
        neuronas[pos aleatoria] = { strength: U(0.5,1)*energy, hue: (bass+treble)%1,
                                    pulse_rate: U(0.05,0.2), age: 0 }
        energy_field[pos] = energy

for pos, n in neuronas:                                 # muertes
    n.age += 1
    if (n.age > 100 and random() < 0.02) or (n.strength < 0.1 and random() < 0.1):
        borrar pos

if random() < treble*0.3:                               # sinapsis nuevas
    conectar dos neuronas al azar, strength U(0.3,1)*mids
synapses = [s for s in synapses if s.strength > 0.1]

# --- difusión de energía (L201-216) ---
new[y][x] = (E[y-1][x] + E[y+1][x] + E[y][x-1] + E[y][x+1]) * 0.23 + E[y][x] * 0.08
energy_field = new * (0.95 + 0.05*energy)

# --- pulso de cada neurona (L219-223) ---
E[y][x] += strength * ((sin(time_counter*pulse_rate)+1)/2) * 0.2

# --- blob de graves (L226-236) ---
if bass > 0.2:
    radius = int(min(w,h) * 0.3 * bass)
    dentro del círculo: E += (1 - dist/radius) * bass * 0.5

# --- onda (L239-261) ---
new[y][x] = (W[y-1] + W[y+1] + W[y][x-1] + W[y][x+1]) * 0.25 - W[y][x]
new[y][x] *= 0.99
wave_field = new + wave_field * 0.5
wave_field += energy_field * 0.03
wave_field = clip(wave_field, -1, 1)

# --- pensamientos (L264-285) ---
x += vx; y += vy; life -= 1
E[pos] += 0.2*size;  W[pos] += 0.1

# --- sinapsis (L288-317) ---
active = 0.8*active + 0.2*(strength_inicio * pulso * strength_sinapsis)
for i in 0..max(|dy|,|dx|)+1:                          # línea interpolada
    E[punto] += active * 0.05
# si una neurona murió: strength *= 0.9
```

Dibujo (L340-380):

```python
for y, x:
    wave = (wave_field[y][x] + 1) / 2
    if energy_field[y][x] < 0.05 and wave < 0.1: continue
    I = min(1.0, energy_field[y][x]*2 + wave*0.5)
    if I < 0.05: continue
    hue = ((x/w + y/h)/2 + hue_offset + bass*0.2 + mids*0.1 + treble*0.05) % 1
    sat = 0.7 + 0.3*I;  val = min(1.0, 0.5 + 0.5*I)
    ch  = symbols[int(I * 19)]
    draw(y+1, x, ch, hsv(hue,sat,val) | (BOLD if I > 0.7 else 0))
```

Y después: sinapsis activas (L383-426, pulsos que viajan por la línea con
`(t + time*0.05) % 1`), neuronas (`❂`/`☼`/`○` por intensidad), pensamientos
(`✺`/`✧`) y una barra "de conciencia" en la última fila con `░▒█` (L486-509).

**Audio** `bass` 22..194 Hz (blob central + color), `mids` 215..624 (resonancia),
`treble` 646 Hz+ (conciencia y sinapsis). El `treble/(bass+0.01)` puede llegar a
50 → `consciousness` se dispara y nunca se estabiliza. Port: clampear a 3.

**Adaptación a 64 bandas** `bass = mean(0,18)`, `mids = mean(18,30)`,
`treble = mean(30,64)`. La puerta del beat (`energy > 0.25 && frame - last > 5`)
pasa a `a.beat` (spote ya tiene onset real) — el contador de frames es un bug
latente: a 30 fps el "cooldown" dura el doble en tiempo real.

**Color** `ramp_at(0.12 + 0.85*I)`; el `hue` posicional pasa a
`ramp_variant(pal, t, h)` (§2.2) con `h = (x/w + y/h)/2`. Las sinapsis: un paso
por encima del campo (`t + 0.15`) para que se distingan. Las neuronas:
`ramp_at(0.75 + 0.2*pulse)` con BOLD. **Nada de 19 glifos exóticos**: ver Anexo A.

**Coste** 3 pasadas full-screen (difusión, onda, dibujo) + sinapsis + partículas:
**~11 400 actualizaciones** de celda y ≤3 800 escrituras a 100×38. Rust: ~150 µs.
Python: ~120 ms (por eso el lite existe).

**Rect pequeño** `area < 1200` → difusión y onda **cada 2 frames** (el campo
cambia lento; nadie lo nota) y el dibujo cada frame. Bajo 600 celdas, usar el
mecanismo de `neural-lite` (3.10) con downsample.

**Port a Rust**

- `field` no se usa: fuera.
- **`dict[(y,x)]` → arrays densos**. `active_neurons` es el punto más caro de
  portar mal. Opciones, de mejor a peor:
  1. `Vec<Neuron>` (slab) + `Vec<u16>` de `w*h` con índice+1 (0 = vacío). 24 bytes
     por neurona, sin hashing, iteración linear. **Recomendado.**
  2. `Vec<Option<Neuron>>` de `w*h`: 4000 × 28 bytes = 112 KB, y el `None` ocupa
     sitio. Aceptable pero desperdicia caché.
  3. `HashMap<(u16,u16), Neuron>`: no. Es lo que hace el original y es lo que lo
     mata.
- **Doble buffer** para `energy_field` y `wave_field` (4 `Vec<f32>`), con
  `std::mem::swap`. Ojo: el original crea `new_energy` y `new_wave` cada frame
  (`np.zeros_like`).
- **Sinapsis**: `Vec<Synapse>` con `max 256`; al morir una neurona, no borrar la
  sinapsis, solo `strength *= 0.9` (como el original) y filtrar de vez en cuando
  (cada 60 frames) en vez de cada frame.
- `time_counter` debe ser `f32` en segundos (`ctx.time`), y todos los
  `sin(time*pulse_rate)` funcionan igual. El original usa frames; a 60 fps
  `pulse_rate = 0.05` = 3 rad/s.
- Los campos son `(h-2)×w` porque el original reserva la fila 0 para el título y
  la última para la barra. En el puerto son `rect.h × rect.w` exactos.

---

### 3.10 `neural-lite` — Neural Dreamscape Lite

**Origen** `visualizers/neural_dreamscape_lite.py`, `update_neural_field` L180-525,
`get_display_color` L527-550, `draw` L552-862 · **Rol** *no* es "el mismo más
rápido": tiene cuatro mecanismos que el original no tiene y que merecen portarse
al neural canónico.

**Qué se ve** Lo mismo pero con puntos cada 2 celdas, y el patrón de las neuronas
cambia según la banda dominante: círculo en el centro con los graves, línea
horizontal con los medios, chispas en los bordes con los agudos.

**Estado y parámetros** Igual que `neural` más:

| Campo | Default | Notas |
|---|---|---|
| `density` | 0.5 | 0.5 / 1.0 (`d`), controla la cuota de puntos |
| `color_mode` | 0 | 0 posición, 1 energía, 2 complementario (`c`) |
| `particle_mode` | 0 | 0 normal, 1 swirl, 2 caos (`p`) |
| `beat_memory` | `[0]*8` | historial de beats |
| `active_neurons` | dict → `{strength, hue, pulse_rate, age, **type**}` | type 0/1/2 |

Los cuatro mecanismos que hay que llevarse:

**1. Downsample 2×** (L111-137). Los campos son `(h/2)×(w/2)` y se pintan en
`(ds_y*2, ds_x*2)` (L631-635). Coste 1/4, aspecto idéntico porque los glifos son puntos
sueltos.

**2. Neuronas tipadas por banda dominante** (L216-290):

```python
dominant = max(bass, mid_low, mid_high, treble)     # compara VALORES, no índices
if dominant == bass:                                 # type 0: círculo central
    r = 0.15*min(w,h);  n = int(3*energy)
    pos = centro + random en el disco
    hue = 0.0;  pulse_rate U(0.05,0.15)
elif dominant in (mid_low, mid_high):                # type 1: línea horizontal
    n = int(4*energy);  step = w // (n+1)
    x = (i+1)*step;  y = h/2 + U(-0.15,0.15)*h
    hue = 0.3;  pulse_rate U(0.1,0.2)
else:                                                # type 2: bordes
    n = int(5*energy)
    pos en uno de los 4 bordes (bandas del 20 % exterior)
    hue = 0.6;  pulse_rate U(0.15,0.25)
```

Vida y muerte por tipo (L306-321): `max_age` 40 / 60 / 80 y probabilidad de
muerte 0.08 / 0.05 / 0.03. Emisión por tipo (L389-425): type 0 pulso central
`×0.3`; type 1 pulso en radio 2 con caída lineal `×0.1`; type 2 chispa aleatoria
en radio 1..3 con prob 0.2 y `×0.4`.

**3. Difusión submuestreada** (L355-383):

```python
grid = 4
offset_y = time_counter % grid
offset_x = (time_counter // 2) % grid
for y in range(offset_y, h, 4):
    for x in range(offset_x, w, 4):
        new[y][x] = mean(4 vecinos válidos) * 0.8 + E[y][x] * 0.1
energy_field *= 0.9
for los mismos puntos: energy_field[y][x] += new[y][x] * 0.1
```

1/16 del trabajo, con el offset rotando para que no se vean las costuras. La
misma idea sirve en el neural completo.

**4. Dibujo por cuota** (L592-628):

```python
quota = min(ds_h*ds_w*density, 300)
# 1º los puntos con E > 0.5
# 2º todas las neuronas activas
# 3º muestra aleatoria de los puntos con E > 0.1 hasta llenar la cuota
```

Techo duro de 300 escrituras. **Este es el truco que hace el efecto viable en un
rect pequeño**, y es reutilizable en todos los efectos de campo.

**Algoritmo (resto)** Igual que 3.9 con las variantes: `bass=mean[:6]*2`,
`mid_low=mean[6:12]`, `mid_high=mean[12:20]`, `treble=mean[20:]`; beat si
`energy > 0.3` con cooldown de 10 frames y `beat_memory` desplazado; rizos de
graves (L426-446) con `ripple_count = int(bass*8)` anillos concéntricos de radio
`(i/count)*0.4*min*bass` y hasta 20 puntos por anillo (proporcional a la
circunferencia); sinapsis solo a neuronas a menos de `w/4`.

**Audio** Cuatro bandas en vez de tres: `bass` 22..130 Hz, `mid_low` 130..258,
`mid_high` 258..430, `treble` 430 Hz+. Con 64 bandas: `mean(0,10)`, `mean(10,16)`,
`mean(16,22)`, `mean(22,64)`.

**Color** `get_display_color` tiene 3 modos; se conservan como estilos, pero con
la rampa:

- modo 0 (posición): `ramp_variant(t, (x+y)/40)`.
- modo 1 (energía): `ramp_at(t)` donde `t = 0.15 + 0.8*energia`.
- modo 2 (complementario): `ramp_variant(t, if I > 0.5 {1.0} else {0.0})`.

**Coste** `(h/2)(w/2)` = 950 celdas de campo (1/16 de difusión = 60 puntos) +
≤300 dibujos + ≤15 sinapsis + ≤30 neuronas. **~1/8 del coste del neural completo.**

**Rect pequeño** Es el modo por defecto cuando `area < 1200`. El downsample 2×
significa que en un rect de 40×12 se dibujan 20×6 puntos: suficiente.

**Port a Rust** Mismo esquema de buffers que 3.9, con `ds = rect.h/2 × rect.w/2`
y `up = 2`. El slab de neuronas necesita un `type: u8` para elegir emisión, vida y
glifo.

---

### 3.11 `pulsar` — Cosmic Pulsar Reactive

**Origen** `visualizers/cosmic_pulsar.py`, `_get_freq_bands` L49-69, `draw` L71-357
· **Rol** el efecto más rico del repo: anillos, pozos gravitatorios, 80 partículas
orbitales y un núcleo pulsante, todo a la vez.

**Qué se ve** Un pulsar central del que salen anillos ondulados con simetría
rotacional; pozos gravitatorios que deforman las órbitas de las partículas; y en
cada beat, más anillos.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `energy_threshold` | 0.3 | 0.05..0.9, paso 0.05 (`t`/`T`) |
| `ring_density` | 1.0 | 0.2..3.0, paso 0.2 (`d`/`D`) |
| `color_mode` | 0 | 0 arcoíris, 1 mono, 2 complementario (`c`) |
| `orbital_count` | 80 | fijo |
| `max_wells` | 4 | fijo |
| `symmetry` | 4 | cicla 2..8 y 0 (`s`) |
| `bass_sensitivity` | 15.0 | comentadas en el original |
| `mids_sensitivity` | 10.0 | idem |
| `highs_sensitivity` | 5.0 | idem |
| `particle_brightness_factor` | 2.5 | fijo |

`Ring { radius, growth_rate, life, thickness, hue, wave_freq, wave_amp, segments, initial_energy }`,
`Well { x, y, base_strength, current_strength, life, lifetime, hue }`,
`Orbital { angle, base_radius, current_radius, orbital_speed, size, freq_idx_rel, hue }`.

**Bandas** (L56-68):

```python
spec_len  = len(spectrum)          # 1024
bass_end  = spec_len // 10         # 102
mids_end  = spec_len // 3          # 341
bass  = min(1.0, mean(spectrum[:102])    * 15.0)   # 0..2 196 Hz
mids  = min(1.0, mean(spectrum[102:341]) * 10.0)   # 2.2k..7.3k
highs = min(1.0, mean(spectrum[341:])    *  5.0)   # 7.3k..22k
```

Fíjate en que el "bass" del pulsar es **10 veces más ancho** que el del resto
(0..2196 Hz, o sea todo hasta los medios) y el "mids" es en realidad el registro
alto. Las sensibilidades 15/10/5 existen porque esas medias lineales son ~10×
más pequeñas que una banda log normalizada.

**Algoritmo**

```python
time += 0.05                                          # por frame

# --- fondo: rejilla con jitter (L84-106) ---
if energy > 0.05:
    for y in range(0, h, 7):
        for x in range(0, w, 14):
            jx = int(sin(time + x*0.1)); jy = int(cos(time + y*0.1))
            if random() < energy*2.0:
                draw(y+jy, x+jx, '.', hsv(offset*0.1, 0.1+energy*0.2, min(0.3, energy*0.5)))

# --- anillos nuevos (L111-132) ---
if energy > threshold and random() < (energy - threshold)*1.5:
    for _ in range(int(1 + energy*2*ring_density)):
        rings.append({ radius: 1,
                       growth_rate: 0.4 + energy*1.2,
                       life: 1.0,
                       thickness: clamp(1 + int(bass*3), 1, 4),
                       hue: random(),
                       wave_freq: 4 + mids*8,
                       wave_amp:  highs*2,
                       segments: randint(sym, sym*3) if random() > 0.7 else 0,
                       initial_energy: energy })

# --- pozos (L136-161) ---
if len(wells) < 4 and random() < (0.005 + energy*0.01 + bass*0.08):
    angle = U(0,2pi); dist = U(0.1, 0.3)*max(w,h)
    wells.append({ x: cx + dist*cos(angle), y: cy + dist*sin(angle),
                   base_strength: U(0.5,2), life: 1.0, lifetime: U(5,15), hue: random() })
for wl in wells:
    wl.life -= (1/wl.lifetime) / 30.0                 # ~30 fps asumidos
    wl.current_strength = wl.base_strength * (0.8 + bass*1.5)

# --- dibujar anillos (L165-222) ---
for r in rings:
    r.radius += r.growth_rate
    r.life   -= 0.015
    if r.life <= 0 or r.radius > max(w,h)*1.5: drop
    brightness = clamp(r.life*0.4 + r.initial_energy*0.6, 0, 1)
    sat = 0.5 + 0.5*brightness;  val = 0.4 + 0.6*brightness
    steps  = max(10, int(2*pi*r.radius))
    stride = max(1, int(steps / (100 + mids*100)))
    for i in range(0, steps, stride):
        angle = i * 2pi / steps
        off   = r.wave_amp * sin(angle*r.wave_freq + time*3)
        rad   = r.radius + off
        for s in range(symmetry):
            a = angle + s * (2pi/symmetry)
            for k in range(r.thickness):
                rr = rad - k
                if rr <= 0: continue
                draw(cy + rr*sin(a), cx + rr*cos(a), '*' if k < thickness/2 else '.')

# --- pozos dibujo (L226-261) ---
intensity = well.life * well.current_strength
chars = '·∘○◎●';  idx = min(4, int(intensity * 3.33))
draw(x, y, chars[idx]) + 2 anillos de '·' con val*(1-r/3)*(0.5+glow*0.5)

# --- partículas orbitales (L266-318) ---
for p in orbitals:
    p.angle = (p.angle + p.orbital_speed * (1 + energy*1.5)) % 2pi
    fe = spectrum[int(p.freq_idx_rel * 1023)]
    p.current_radius = p.base_radius * (1 + fe*2*0.1)
    px = cx + p.current_radius*cos(p.angle);  py = cy + p.current_radius*sin(p.angle)
    for wl in wells:                                  # deformación gravitatoria
        vx = wl.x - px; vy = wl.y - py
        d2 = vx*vx + vy*vy;  d = sqrt(d2) if d2 > 0.01 else 0.1
        f  = min(wl.current_strength * wl.life / d2 * 3, d/2)
        px += vx/d * f;  py += vy/d * f
    val = clamp(0.4 + fe*2.5, 0.1, 1);  sat = clamp(0.6 + fe*0.4, 0.2, 1)
    draw(py, px, '#' if fe > 0.5 else p.size)

# --- núcleo (L321-356) ---
cr = 1.0 + energy*5.0
ci = clamp(0.2 + energy*0.8, 0, 1)
for layer in range(int(cr), -1, -1):
    I = ci * max(0, 1 - layer/(cr + 1e-6))**2
    if I < 0.05: continue
    ch = '*' if layer < cr/2 else '+' if layer < cr*0.8 else '.'
    for i in range(max(6, int(layer*4))):
        a = i*2pi/steps; draw(cy + layer*sin(a), cx + layer*cos(a), ch)
draw(cy, cx, 'O')                                     # centro siempre
```

**Detalles que el código deja a medias**

- `segments` se calcula, se guarda y **nunca se usa** (hay un `# TODO` en L174).
  Estado muerto: fuera.
- `color_mode == 2` hace `hue_offset + 0.5*(ring.hue > 0.5)`: un **booleano**
  multiplicado, no un complementario de rueda. Son dos colores, no dos
  complementarios.
- `well.life -= (1/lifetime)/30.0` asume 30 fps. A 60 fps los pozos duran la
  mitad en tiempo real.
- Los anillos no se autolimitan: como se crean en cada frame con
  `energy > threshold`, un tema con energía alta constante llena la pantalla de
  anillos.

**Audio** `bass` (0..2.2 kHz) decide grosor, tamaño del núcleo y fuerza de los
pozos; `mids` (2.2..7.3k) decide la densidad de muestreo y la frecuencia de la
onda del anillo; `highs` (7.3k+) la amplitud de esa onda. Y `energy` decide todo
lo demás.

**Adaptación a 64 bandas** **No** traducir `[:102]` a `bands[0..50]`: eso mete
todo el espectro en "bass". Usar tercios con ganancias calibradas:

```rust
let bass  = (mean(&a, 0, 6)  * 1.6).min(1.0);   // 40-100 Hz
let mids  = (mean(&a, 6, 21) * 1.4).min(1.0);   // 100-380 Hz
let highs = (mean(&a, 21, 64) * 1.8).min(1.0);  // 380 Hz-16 kHz
```

y validar con la comprobación de §1.3. El `particle_brightness_factor` de 2.5
funciona igual sobre `bands[0..64]` mapeado a `freq_idx_rel`.

**Color** Con la rampa, los tres modos quedan:

- `Mono`: `ramp_at(0.10 + 0.85*brightness)` para anillos, pozos y núcleo. Es el
  modo "portada monocroma" y el que mejor va detrás del arte.
- `Album`: `ramp_variant(t, ring.hue)`; las partículas usan `ramp_variant(t, freq_idx_rel)`
  (el grave tira a `primary`, el agudo a `secondary`).
- `Dual`: `ramp_variant(t, if hue > 0.5 {1.0} else {0.0})`.

El núcleo siempre `ramp_at(1.0)` (o `text`) y el centro `'O'` con saturación 0 →
`text` puro.

**Coste** El más caro después del fractal, y el más difícil de acotar:

- Anillos: `rings × steps × symmetry × thickness`. Con 8 anillos, radio medio 40,
  `steps = 251`, `stride = 1` (mids alto), `symmetry = 4`, `thickness = 3` →
  **24 000 escrituras/frame**.
- Partículas: 80 × 4 pozos = 320 iteraciones + 80 escrituras.
- Núcleo: ~100. Fondo: 35. Pozos: 4 × (1 + 2 anillos × 8 puntos).

**Rect pequeño** `min(w,h) < 16` → `symmetry = 0` (sin simetría), `thickness = 1`,
sin pozos y sin fondo. `area < 800` → `max_rings = 3`.

**Port a Rust**

- `max_rings = 8` y `symmetry` por defecto 4 (no 6). Con stride
  `(steps / (80 + mids*60))` → 80..140 muestras por anillo, no 250.
- LUT de seno/coseno **obligatoria** (2 por punto).
- Estado en `Vec<Ring>` con capacidad 8, `Vec<Well>` con 4, `[Orbital; 80]` fijo
  (nunca cambian de número: se crean una vez en `init`).
- Las ganancias (`bass_sensitivity` etc.) se van a `Params` de solo lectura; no
  son teclas.
- `time += 0.05` → `ctx.time` (radianes/segundo reales, ~3 rad/s).
- Si va **detrás** del arte: `Mono`, cap de rampa 0.5, sin BOLD, y sin fondo de
  rejilla (el arte ya da textura).

---

### 3.12 `stick` — Dancing Stick Figure

**Origen** `visualizers/stick_figure.py`, `draw` L16-146, `draw_line` L148-172 ·
**Rol** el efecto "gracioso". Barato, pero mal encajado en un compositor.

**Qué se ve** Un muñeco de palitos bailando en el centro, con una pista en el
suelo que se ensancha con la energía.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `dance_mode` | 0 | 0 normal, 1 breakdance, 2 robot (`d`) |
| `speed` | 0.5 | 0.1..1.5, paso 0.1 (`s`/`S`) |
| `phase` | 0 | `+= energy * speed` por frame |

**Algoritmo**

```python
# 1. BORRADO (L22-26): escribe ' ' en un rect 30x20 centrado
for y in (cy-10)..(cy+10):
    for x in (cx-15)..(cx+15):
        draw(y, x, ' ')

phase += energy * speed

# 2. Pose según dance_mode (L32-111)
# modo 0 (normal):
head  = (cx, cy-6)
body  = (cx, cy+1)
l_arm = (cx - 4cos(phase),      cy - 3 - sin(phase))
r_arm = (cx + 4cos(phase+pi),   cy - 3 - sin(phase+pi))
l_leg = (cx - 3cos(0.7*phase),  cy + 5 + sin(0.7*phase))
r_leg = (cx + 3cos(0.7*phase+pi), cy + 5 + sin(0.7*phase+pi))
# modo 1 (breakdance): head = (cx+4cos p, cy-2-2sin p)
#   brazos con phase*2 y phase*2+pi/2, radio 5
#   piernas con phase*1.5 y phase*1.5+pi/3, radio 5, y +3+3sin
# modo 2 (robot): head con jitter random si energy > 0.5 / 0.7
#   arm_phase = int(phase*4) % 4  -> 4 posiciones tabuladas
#   leg_phase = int(phase*2) % 2  -> 2 posiciones tabuladas

# 3. Dibujo
draw(head, 'O', BOLD)
for i in 1..5: draw(head_y+i, cx, '|')
draw_line(cx, head_y+2, l_arm, '/')      # ¡ojo, ver nota!
draw_line(cx, head_y+2, r_arm, '\\')
draw_line(cx, body_y,   l_leg, '/')
draw_line(cx, body_y,   r_leg, '\\')

# 4. Suelo (L138-146)
floor_y = body_y + 6;  fw = int(15 + 10*energy)
for x in (cx-fw)..(cx+fw): draw(floor_y, x, '_', hsv(offset + (x-cx)/fw/2, 0.7, 0.8))
```

`draw_line` es Bresenham estándar (L148-172), correcto.

**El borrado es el problema del port.** En un compositor de capas, "borrar" es
imposible sin destruir lo que hay debajo. Dos opciones:

1. **No borrar** (recomendado): el muñeco son ~40 celdas sueltas; al no borrar,
   quedan estelas del frame anterior. Como los brazos se mueven poco, apenas se
   nota, y el efecto gana un rastro que le sienta bien.
2. Mantener un "frame anterior" de las celdas que pintó y limpiarlas a `None`
   (solo válido si la capa es la única que escribe en ese rect). 40 posiciones,
   trivial, pero rompe si hay otra capa debajo.

**El glifo de los brazos no coincide con la dirección.** L131-136 pasa `/` para el
brazo izquierdo y `\` para el derecho, independientemente de hacia dónde vayan.
Cuando `left_arm_x < cx` la línea sube a la izquierda y debería ser `\`. En el
port:

```rust
let ch = match (dx > 0, dy < 0) {
    (true,  true)  => '/',   // sube a la derecha
    (false, true)  => '\\',  // sube a la izquierda
    (true,  false) => '\\',  // baja a la derecha
    (false, false) => '/',   // baja a la izquierda
};
```

**Audio** Solo `energy`, para `phase` y para el ancho del suelo. No usa bandas.
Con `speed = 0.5` y `energy = 0.3`, `phase` avanza 0.15 rad/frame = 9 rad/s = 1.4
vueltas/s: baila rápido.

**Adaptación a 64 bandas** Añadir reacción al beat que el original no tiene:
`phase += (energy*0.5 + beat_pulse*1.5) * speed * dt * 60`, y en modo 2
(robot) usar `beat` para el salto de pose en vez de `int(phase*4)`.

**Color** `ramp_at(0.55 + 0.40*energy)`, plano para toda la figura (como el
original). El suelo: `ramp_at(0.25 + 0.35*((x-cx)/fw + 1)/2)`, un degradado
horizontal.

**Coste** ~**80 escrituras** sin el borrado (600 con él). El efecto más barato de
todos en dibujo, y el más caro en "celdas desperdiciadas" si se borra.

**Rect pequeño** `h < 16 || w < 20` → no cabe la figura completa: dibujar solo
cabeza + suelo. `h < 12` → no dibujar.

**Port a Rust** Sin el clear; glifo por dirección; `dance_mode` como parámetro
(ciclo con la tecla del panel de efectos, no con `d`). Es el candidato natural
para ir **delante** del arte en la franja inferior, nunca detrás.

---

### 3.13 `tuner` — Guitar Tuner

**Origen** `visualizers/guitar_tuner.py`, `get_string_frequency` L31-99,
`draw` L101-326 · **Rol** no es un visualizador: es un afinador. Se documenta
porque está en la lista, pero **recomiendo no portarlo** (ver más abajo).

**Qué se ve** Una caja con un arco de marcas (FLAT / IN TUNE / SHARP), una aguja
que se mueve con física de muelle, el desvío en cents, y 6 cuerdas dibujadas con
grosores distintos, la seleccionada vibrando.

**Estado y parámetros**

| Campo | Default | Rango / teclas |
|---|---|---|
| `guitar_strings` | E2 82.41, A2 110.00, D3 146.83, G3 196.00, B3 246.94, E4 329.63 | fijo |
| `string_index` | 0 | 0..5 (`←`/`→`, `a`/`h`, `d`/`l`) |
| `needle_position` | 0.5 | 0.05..0.95 |
| `needle_momentum` | 0 | — |
| `accuracy`, `animation_frames` | 0 | — |

**Algoritmo de detección** (L31-99)

```python
N = 2048; sr = 44100                       # hardcodeados
bin_width = sr / N                         # 21.53 Hz
target_bin = int(target_freq / bin_width)
radius_hz  = target_freq * 0.10            # ±10 %
radius_bin = max(1, int(radius_hz / bin_width))
start, end = clamp(target_bin ± radius_bin, 0, len(spectrum)-1)
strongest  = argmax(spectrum[start..end])
detected   = strongest * bin_width
cents      = 1200 * log2(detected / target_freq)
accuracy   = max(0, 1 - abs(cents)/50)     # ±50 cents = cuarto de tono
position   = clamp(0.5 + cents/100, 0, 1)  # -50c → 0, +50c → 1
```

**Física de la aguja** (L115-122)

```python
force    = (position - needle) * 0.2
damping  = -momentum * 0.2
momentum += force + damping
momentum *= 0.85
needle   += momentum
needle    = clamp(needle, 0.05, 0.95)
```

Es un muelle de coeficientes ad-hoc, estable a 60 fps. En el puerto, si se quiere
independencia de fps: `k=120`, `c=18`, integración semi-implícita con `dt`.

**El problema: las 64 bandas no pueden medir afinación.**

- Resolución de banda: `1.0981×` = **162 cents**.
- Tolerancia del efecto: **±50 cents** (ratio 1.029).
- En 82.41 Hz, la banda que contiene esa frecuencia abarca 77.0..84.6 Hz
  (banda 7: `40*1.0981^7` a `40*1.0981^8`).

→ Es 3× más gruesa que la tolerancia. El afinador **no puede funcionar** con
`AudioFrame::bands`. Tres salidas:

1. **No portarlo.** Es lo que recomiendo: spote reproduce música grabada; un
   afinador de guitarra no tiene sitio en una TUI de música.
2. **Exponer el espectro crudo** cuando el efecto está activo:
   `AudioFrame::raw: Option<&[f32]>` (1024 bins, 4 KB). Con eso el algoritmo del
   original funciona, **pero sigue sin ser suficiente**: en 82 Hz un bin son
   21.5/82 = 260 cents. Hay que **interpolar el pico** (esto es obligatorio, el
   original no lo hace y por eso es impreciso):
   ```rust
   let d = 0.5 * (m[k-1] - m[k+1]) / (m[k-1] - 2.0*m[k] + m[k+1]);
   let freq = (k as f32 + d) * bin_width;
   ```
   Precisión resultante: ~5-15 cents. Suficiente.
3. **Autocorrelación sobre `waveform[256]`**: excelente para una cuerda sola,
   basura con música mezclada. No.

**Color** Por zonas de la escala: centro `ramp_at(1.0)` (verde en el original),
zona media `ramp_at(0.6)`, extremos `ramp_at(0.25)`. La aguja, `ramp_at(1.0)` si
`accuracy > 0.9`, si no `ramp_at(0.7)`.

**Coste** O(ventana de búsqueda) ≈ 2·radius_bin ≈ 8 bins. Trivial.

**Rect pequeño** `h < 16 || w < 20` → "Terminal too small!" (L140-142), como el
original.

**Port a Rust** Si aun así se implementa: `Params { string: 0..5, tolerance: 50 }`,
la interpolación parabólica de arriba, y la aguja con spring real. Todo lo demás
(caja, marcas, cuerdas vibrando) es `Block` + bucle de pintado; no tiene
migración de mecanismo, solo de dibujo.

---

## 4. Composición por capas y blend modes

`idea.md` §7.4 ya define `Layer { effect, blend, opacity, mask, enabled }` y
`Viz { layers: Vec<Layer> }` con hasta 6 capas. Esta sección concreta **cómo se
mezcla a nivel de celda**, que es la parte que no está resuelta allí y la que
decide si 3 efectos juntos se ven como una imagen o como una papilla.

### 4.1 Modelo de celda

**No uses `ratatui::Cell` como buffer de capa.** En ratatui 0.30 (`ratatui-core`
0.1.0) `Cell` es `Clone` pero **no `Copy`**: contiene `symbol: Option<CompactString>`.
Copiar o clonar 4000 de ellas por capa y por frame mueve 4000 `CompactString`
(y una comparación de strings por celda en el diff del `Terminal`). El buffer de
capa propio:

```rust
/// 8 bytes. Copy. Sin punteros. memcpy-able.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct VizCell {
    /// Índice a GLYPHS. 0 = celda vacía (no se pinta nada).
    pub glyph: u8,
    /// Densidad visual del glifo, 0..15. Ver 4.3.
    pub ink: u8,
    /// Color del glifo (no del fondo: el fondo lo pone el arte o el tema).
    pub fg: [u8; 3],
}

impl VizCell {
    pub const EMPTY: Self = Self { glyph: 0, ink: 0, fg: [0, 0, 0] };
    #[inline] pub fn is_empty(&self) -> bool { self.glyph == 0 }
}

pub struct LayerBuf {
    pub cells: Vec<VizCell>,   // len = w*h, `EMPTY` al empezar el frame
    pub w: u16,
    pub h: u16,
}

impl LayerBuf {
    /// `fill(EMPTY)` en cada frame: 4000 × 8 B = 32 KB, ~2 µs. No reallocar.
    #[inline] pub fn clear(&mut self) { self.cells.fill(VizCell::EMPTY); }
    #[inline] pub fn set(&mut self, x: u16, y: u16, c: VizCell) {
        let i = y as usize * self.w as usize + x as usize;
        if let Some(slot) = self.cells.get_mut(i) { *slot = c; }
    }
}
```

`u8 glyph` obliga a un LUT global de glifos, que es exactamente lo que queremos
para el Anexo A (todos de ancho 1, verificados una vez al arrancar):

```rust
pub static GLYPHS: &[char] = &[' ', '·', '.', ',', ':', ';', '-', '=', '+',
                               '*', '#', '%', '@', '│', '█', '▄', /* … */];
```

Los efectos **nunca** escriben el fondo (`bg`). El fondo lo controla el arte y el
tema. La única excepción son los glifos de bloque (`█`, `▄`, `▀`), que son
opacos por naturaleza: por eso van con `ink` alto y el compositor sabe tratarlos.

### 4.2 Blend por color

Todo el blending se hace en **luz lineal**, no en sRGB. Un `Screen` de dos medios
en código sRGB da 0.75, que en luz es 0.52: el resultado se ve turbio y
desaturado. El mismo `Screen` en luz lineal da 0.75 de luz (0.88 en código). Con
dos capas se nota poco; con tres, es la diferencia entre "brilla" y "papilla".

```rust
/// LUT de 256 entradas: sRGB u8 → lineal 0..1.
static SRGB_TO_LIN: LazyLock<[f32; 256]>  = LazyLock::new(build_srgb_to_lin);
/// LUT de 1024: lineal 0..1 (indexado por (x*1023.0) as usize) → u8.
static LIN_TO_SRGB: LazyLock<[u8; 1024]>  = LazyLock::new(build_lin_to_srgb);
```

Construirlas en un `LazyLock`/`OnceLock` al arrancar (`powf` solo 1280 veces).
Sin LUT son 6 `powf` por celda y por capa: 4000 × 3 capas × 6 = 72 000 `powf` =
3-5 ms. **Inasumible.** Con LUT: 2 lookups + aritmética, ~15 ns por celda.

```rust
#[inline]
fn blend_channel(mode: Blend, base: f32, top: f32, alpha: f32) -> f32 {
    match mode {
        // Normal: lerp
        Blend::Normal   => base + (top - base) * alpha,
        // Add: suma, con saturación. La opacidad escala la contribución.
        Blend::Add      => (base + top * alpha).min(1.0),
        // Screen: 1-(1-a)(1-b). Nunca oscurece. Ideal sobre el arte.
        Blend::Screen   => 1.0 - (1.0 - base) * (1.0 - top * alpha),
        // Multiply: b*a. Con alpha=0 devuelve base (multiplicar por 1).
        Blend::Multiply => base * (1.0 + (top - 1.0) * alpha),
        // Overlay: contraste. Oscurece por debajo de 0.5, aclara por encima.
        Blend::Overlay  => {
            let o = if base < 0.5 { 2.0 * base * top }
                    else          { 1.0 - 2.0 * (1.0 - base) * (1.0 - top) };
            base + (o - base) * alpha
        }
    }
}

fn blend_rgb(mode: Blend, base: [u8; 3], top: [u8; 3], alpha: f32) -> [u8; 3] {
    if alpha <= 0.0 { return base; }
    let a = alpha.clamp(0.0, 1.0);
    let mut out = [0u8; 3];
    for i in 0..3 {
        let b = SRGB_TO_LIN[base[i] as usize];
        let t = SRGB_TO_LIN[top[i] as usize];
        let v = blend_channel(mode, b, t, a);
        out[i] = LIN_TO_SRGB[(v.clamp(0.0, 1.0) * 1023.0) as usize];
    }
    out
}
```

Notas de cada modo, para elegir con criterio:

| Modo | Qué hace | Cuándo usarlo | Riesgo |
|---|---|---|---|
| `Normal` | sustituye | capas opacas, el arte, `bars` en primer plano | tapa lo de debajo |
| `Add` | suma luz | `flame`, `particles`, núcleos, beats | satura a blanco con 2 capas |
| `Screen` | suma suave | `wave`, `starfield`, `circle` sobre arte | lava los negros si se abusa |
| `Multiply` | multiplica | `matrix` como textura, viñetas | mata los detalles oscuros |
| `Overlay` | contraste | `fractal` sobre arte | impredecible con colores medios |

Regla práctica sobre fondo oscuro (`bg = #0b0b10`): con `Add` y `Screen`, un
`alpha > 0.5` ya domina la celda. Para 3 capas usa `0.35-0.55`, no 0.8.

### 4.3 Fusión de caracteres

El color se mezcla, el **glifo no**: una celda solo puede tener un carácter. La
regla es un z-buffer de tinta:

```rust
fn composite_glyph(under: VizCell, over: VizCell, mode: Blend) -> (u8, u8) {
    // (glyph, ink) resultante
    match mode {
        // Los modos aditivos NO sustituyen el glifo: se queda el más "denso".
        // Así una estrella no borra una barra sólida, y dos capas de texto se
        // suman en lugar de pelearse.
        Blend::Add | Blend::Screen | Blend::Overlay =>
            if over.ink > under.ink { (over.glyph, over.ink) } else { (under.glyph, under.ink) },
        // Multiply: si debajo hay algo, se queda; si está vacío, pinta.
        Blend::Multiply =>
            if under.is_empty() { (over.glyph, over.ink) } else { (under.glyph, under.ink) },
        // Normal: sustituye si hay opacidad suficiente (el alpha se aplica en 4.5).
        Blend::Normal => (over.glyph, over.ink),
    }
}
```

`ink` es la densidad del glifo. Cada efecto la declara al escribir (no se calcula
en el compositor):

| `ink` | Glifos | Uso |
|---|---|---|
| 0 | (vacío) | nada |
| 1-2 | `.` `,` `·` `'` | fondo, polvo, colas de matrix |
| 3-4 | `:` `;` `-` `~` | trazos finos, ondas |
| 5-6 | `=` `+` `*` | estrellas, partículas, cruces |
| 7-8 | `o` `O` `0` `#` | cabezas, nodos |
| 9-11 | `%` `@` `&` | denso, bordes de fractal |
| 12-13 | `│` `─` `▄` `▀` | líneas y medios bloques |
| 14-15 | `█` `▓` `▒` | sólido, barras |

Con esto, `wave + particles + starfield` en `Screen` produce una imagen donde las
tres contribuciones se ven (cada una en su rango de `ink`) en vez de que la
última tape a las otras.

**Caso especial**: `bars` y `flame` escriben `█` (ink 15) y ganan siempre. Es
correcto: si están en una capa, es porque deben dominar. Por eso `flame` va en
`Add` con opacidad 0.9 y no en `Normal`.

### 4.4 Máscaras

```rust
#[derive(Clone, Copy)]
pub enum Mask {
    Full,
    /// Anillo/elipse. `t = dist / (0.5*min(w,h))`, con corrección de aspecto.
    Radial { r_in: f32, r_out: f32, invert: bool, aspect: f32 },
    /// Degradado vertical. `t = y / h` (0 arriba, 1 abajo).
    Vertical { from: f32, to: f32, gamma: f32 },
    /// Solo se enciende cuando esa banda del espectro suena.
    Band { lo: usize, hi: usize, gain: f32, floor: f32 },
    /// Rectángulo duro (para confinar un efecto a una esquina).
    Rect(Rect),
}

fn mask_value(m: Mask, x: u16, y: u16, w: u16, h: u16, a: &AudioFrame) -> f32 {
    match m {
        Mask::Full => 1.0,
        Mask::Radial { r_in, r_out, invert, aspect } => {
            let cx = w as f32 * 0.5; let cy = h as f32 * 0.5;
            let dx = (x as f32 - cx) / aspect;      // aspect = CELL_ASPECT
            let dy = y as f32 - cy;
            let r  = (dx*dx + dy*dy).sqrt() / (0.5 * (w as f32).min(h as f32 * aspect));
            let v  = 1.0 - smoothstep(r_in, r_out, r);   // 1 dentro, 0 fuera
            if invert { 1.0 - v } else { v }
        }
        Mask::Vertical { from, to, gamma } => {
            let t = y as f32 / h as f32;
            let v = smoothstep(from, to, t);
            v.powf(gamma)
        }
        Mask::Band { lo, hi, gain, floor } => {
            let e = mean(a, lo, hi) * gain;
            floor + (1.0 - floor) * smoothstep(0.0, 1.0, e)
        }
        Mask::Rect(r) => if r.contains(Position::new(x, y)) { 1.0 } else { 0.0 },
    }
}

#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
```

Máscaras que valen la pena, con parámetros concretos:

| Máscara | Uso | Valores típicos |
|---|---|---|
| `Radial { 0.0, 1.0 }` | el efecto solo en el centro (detrás de la portada) | `r_in=0.0, r_out=1.0` |
| `Radial { 0.35, 1.0, invert }` | viñeta: solo en los bordes | `invert=true` |
| `Vertical { 1.0, 0.0, 1.6 }` | el efecto se apaga hacia arriba (barras, llama) | fondo sólido abajo |
| `Vertical { 0.0, 0.7, 1.0 }` | se apaga hacia abajo (estrellas, matrix) | |
| `Band { 0, 18, 1.0 }` | solo late con el bombo | `lo=0, hi=18` |
| `Band { 44, 64, 1.2 }` | solo con los hats/brillo | `lo=44, hi=64` |

La máscara `Band` es la que hace que un efecto "respire" con una parte del
espectro sin programarlo dentro del efecto. Combinada con `Add` da el efecto
"el visualizador se enciende con el bombo" sin tocar el código del efecto.

### 4.5 Opacidad y orden

```rust
fn compose(dst: &mut [VizCell], src: &LayerBuf, layer: &Layer,
           a: &AudioFrame, rect: Rect) {
    let alpha0 = layer.opacity.clamp(0.0, 1.0);
    for y in 0..rect.h {
        for x in 0..rect.w {
            let over = src.cells[(y * src.w + x) as usize];
            if over.is_empty() { continue; }
            let m = mask_value(layer.mask, x, y, rect.w, rect.h, a);
            let alpha = alpha0 * m;
            if alpha <= 0.004 { continue; }
            let idx = (y * rect.w + x) as usize;
            let under = dst[idx];
            let (g, ink) = composite_glyph(under, over, layer.blend);
            // Una celda vacía no tiene color: el blend parte del fondo del tema.
            let base_fg = if under.is_empty() { a.palette.bg_24() } else { under.fg };
            let fg = blend_rgb(layer.blend, base_fg, over.fg, alpha);
            dst[idx] = VizCell { glyph: g, ink, fg };
        }
    }
}
```

Detalles:

- `> 0.004` es la opacidad mínima para que el blend cambie algo en 8 bits.
  Descartar antes de hacer las 6 lookups de la LUT ahorra la mitad del coste en
  capas con máscara radial.
- Si la capa está vacía en una celda (`glyph == 0`), **no se toca** la de debajo
  (esto es lo que permite que un efecto con 300 puntos no rellene el buffer).
- El orden es de atrás a delante. Las capas `behind_art` se componen **antes** del
  arte; las demás, después.
- `base_fg` es necesario porque una celda vacía tiene `fg = [0,0,0]`; usar 0 como
  color base en `Screen`/`Multiply` da resultados correctos por casualidad, pero
  en `Normal` y `Overlay` sale negro. `Palette::bg` es un `(u8,u8,u8)`; basta un
  `[bg.0, bg.1, bg.2]` (`bg_24()` en el pseudocódigo).

**Presupuesto.** El compositor cuesta `rect.area() × n_capas_activas` iteraciones,
pero solo en las celdas con `glyph != 0`. Regla de oro: **un frame completo
(efectos + composición) debe caber en 8 ms**, que es el umbral de `idea.md`
§7.4. A 100×40 y 3 capas eso son ~2.8 M de operaciones de celda/s de margen: sobra
siempre que los efectos respeten las cotas de §3.

### 4.6 Legibilidad sobre el arte ASCII

El arte de la portada (`ArtCell { ch, fg, bg }`, con `bg` opcional) es una capa
más, pero con una propiedad que los efectos no tienen: **puede ser denso y
brillante**. Reglas:

**A. Efecto detrás del arte.** El arte manda; el efecto es textura.

```rust
// art_ink: densidad del glifo del arte en esa celda, 0..1 (espacio = 0, █ = 1)
// art_luma: luma del fg del arte en esa celda, 0..1
let dim = 1.0 - 0.75 * art_ink_for_effects;   // 1 en zonas vacías, 0.25 donde hay tinta
let color = scale(viz_color, dim);
// y además, nunca BOLD, y techo de rampa 0.55 (ver §2.4)
```

Con esto, un `matrix` a pantalla completa detrás de la portada sigue leyéndose en
el margen y desaparece bajo las zonas densas del arte. Es el efecto "ventana
empañada" y es exactamente lo que se quiere.

**B. Efecto delante del arte.** El efecto manda, pero el arte debe seguir
reconociéndose.

```rust
// Si el arte es claro, el efecto se pinta oscuro (y al revés). No es invertir
// colores: es quedarse en el extremo opuesto de la rampa.
let t = if art_luma > 0.45 { 0.12 } else { 0.92 };
let color = pal.ramp_at(t);
```

Alternativa mejor para glifos sueltos (partículas, estrellas) que para glifos
densos: **doble trazo**. Pintar el glifo en `bg_deep` si el arte local es claro,
o en `text` si es oscuro, y dejar que el `ink` del arte haga de contraste. Cuesta
1 celda extra por punto.

**C. Contraste mínimo.** Antes de escribir, `Palette::contrast(color, art_fg)`
debe ser ≥ 2.0. Si no, subir un paso de rampa (máx. 3 intentos) o descartar la
celda. Esto evita el caso clásico: partículas `primary` sobre arte `primary`.

**D. El arte nunca se compone con `Multiply`.** Multiplicar el arte por un efecto
lo convierte en una mancha. `Screen` como mucho, y con `alpha < 0.25`.

**E. `bg` del arte.** `ArtCell.bg` es `Option<Rgb>`: en modo `Half` el arte usa
medio bloque con color arriba/abajo y **ocupa el fondo**. Un efecto detrás de un
arte en modo `Half` no se ve: hay que pasar a `Dither` o `Braille`, o poner el
efecto en la franja de abajo. Documentarlo en la UI.

### 4.7 Cinco presets

Cada preset es una lista de capas con `blend`, `opacity`, `mask` y `behind_art`.
Nombres cortos, sin épica. Todos asumen `CELL_ASPECT = 2.0` y la paleta del
álbum.

**1. `aurora`** — el "todo suave". Fondo por defecto, funciona con cualquier
portada. Pensado para detrás del arte.

| # | efecto | blend | opacity | mask | behind |
|---|---|---|---|---|---|
| 1 | `starfield` | `Screen` | 0.35 | `Vertical { 0.0, 0.8, 1.0 }` | sí |
| 2 | `wave` | `Screen` | 0.75 | `Full` | sí |
| 3 | `particles` | `Add` | 0.55 | `Vertical { 0.35, 1.0, 1.4 }` | sí |

Coste: ~2 700 celdas de las cuales 2 500 son de `wave` (1 000) y `starfield`
(150) — el resto es composición. Con `screen` el arte se ve a través.

**2. `inferno`** — el "hay bombo". Delante del arte, abajo.

| # | efecto | blend | opacity | mask | behind |
|---|---|---|---|---|---|
| 1 | `flame` | `Add` | 0.90 | `Vertical { 0.55, 1.0, 1.2 }` | sí |
| 2 | `bars` | `Add` | 0.65 | `Band { 0, 18, 1.4, 0.15 }` | no |
| 3 | `particles` | `Add` | 0.40 | `Vertical { 0.7, 1.0, 2.0 }` | no |

La máscara `Band` en `bars` hace que las barras solo aparezcan **cuando suena el
bombo**, con un suelo de 0.15 (nunca del todo invisibles). Es el truco que más
"vida" da por línea de código.

**3. `void`** — el "matrix de fondo". Detrás del arte, oscuro, no compite.

| # | efecto | blend | opacity | mask | behind |
|---|---|---|---|---|---|
| 1 | `matrix` | `Multiply` | 0.45 | `Rect` (mitad derecha) | sí |
| 2 | `starfield` | `Screen` | 0.25 | `Full` | sí |

`Multiply` con `matrix` es lo único que hace legible el efecto sobre el arte: las
columnas oscurecen el arte en vez de taparlo, así que se lee como textura. La
mitad derecha evita que se coma la portada.

**4. `bloom`** — el "pulsar limpio". Detrás del arte, centrado.

| # | efecto | blend | opacity | mask | behind |
|---|---|---|---|---|---|
| 1 | `pulsar` (`Mono`) | `Add` | 0.50 | `Radial { 0.0, 0.9 }` | sí |
| 2 | `circle` | `Screen` | 0.70 | `Radial { 0.15, 1.0, invert: true }` | sí |
| 3 | `wave` | `Screen` | 0.35 | `Vertical { 0.6, 1.0, 1.5 }` | sí |

`pulsar` en modo `Mono` con `Add` y 0.5 es lo más bonito del conjunto: los
anillos se suman al arte sin taparlo.

**5. `glitch`** — el "no es para todos". Delante del arte, solo agudos.

| # | efecto | blend | opacity | mask | behind |
|---|---|---|---|---|---|
| 1 | `fractal` (step 2) | `Overlay` | 0.40 | `Band { 40, 64, 1.6, 0.0 }` | sí |
| 2 | `bars` | `Add` | 0.70 | `Band { 44, 64, 1.8, 0.0 }` | no |
| 3 | `flame` | `Add` | 0.30 | `Band { 0, 16, 1.2, 0.0 }` | sí |

El `fractal` solo aparece con los agudos y las barras solo con los hats: el
efecto parpadea con la música de forma agresiva. Con `Overlay` sobre el arte es
impredecible a propósito.

Y un sexto, para cuando no hay tiempo: **`clean`** = solo `bars`, `Normal`,
opacity 1.0, `Full`. Es el modo por defecto y el que se usa en terminales de
menos de 90 columnas.

**Presets en la UI**: tecla `S` cicla; el nombre y las capas se muestran 1.5 s en
el footer al cambiar. Los presets 1-4 van detrás del arte; el 2 y el 5 mezclan.
Ninguno usa más de 3 capas: 4 ya no se distingue y cuesta el doble.

**Estado en el código.** `viz/mod.rs` ya trae `PRESETS` con siete nombres
(`aurora, inferno, void, bloom, glitch, clean, neural`) y combinaciones distintas
de las de arriba. Los cinco cambios que merece la pena hacer, por orden de
impacto:

1. **`void`: `matrix` con `Add` 0.55 es justo lo que hay que evitar.** `Add` sobre
   el arte lo lava; `Multiply` 0.45 lo usa como textura y el arte se sigue
   leyendo. Además, confinarlo a media pantalla (`Mask::Band(0.5, 1.0)`).
2. **Falta la máscara de frecuencia.** `Mask::Band(t0, t1)` es vertical del
   canvas; hace falta una que multiplique por el nivel de `bands[lo..hi]`. Es la
   única pieza de este documento que, sola, mejora todos los presets: hace que
   cualquier capa "respire" con el bombo sin tocar el efecto.
3. **`inferno`: `bars` en `Normal` + `Mask::Bottom` tapa el arte.** Pasa a `Add`
   0.55-0.65; cuando exista la máscara de frecuencia, úsala con `lo=0, hi=18`.
4. **`bloom`: `pulsar` a `Add` 0.85 satura a blanco** en los 3-4 frames
   siguientes a un beat (su `Add` es sobre `ramp_at` alto, no sobre negro).
   Baja a 0.5 y sube `circle` a `Screen` 0.7.
5. **`glitch`: `fractal` en `Normal` 0.85 tapa el arte.** `Overlay` 0.40 se lee
   como textura; con máscara de agudos (`lo=44, hi=64`) además parpadea con los
   hats en vez de estar siempre ahí.

---

## 5. Prioridad de implementación

Ordenado por **impacto visual / coste**, y además por lo que cada uno **valida de
la infraestructura**. Los cuatro primeros son obligatorios antes de tocar nada
caro: si `bars`, `wave`, `particles` y `starfield` van finos a 60 fps con 2 capas,
el resto es trabajo mecánico.

`impacto` y `coste` en escala 1-5 (5 = máximo). `celdas/frame` es el peor caso a
100×40, antes de composición.

| # | Efecto | Impacto | Coste | Celdas/frame | Por qué aquí |
|---|---|---|---|---|---|
| 1 | **`bars`** | 4 | 1 | ≤3 600 | Valida el pipeline entero con 40 líneas: `AudioFrame` → `LayerBuf` → compositor → `ramp_at`. Si esto se ve bien, la mitad del sistema está hecha. Además es el único efecto que sirve como *fallback* universal. |
| 2 | **`wave`** | 3 | 1 | ≤100 (2 000 `sin`) | Valida `ctx.time`, la pausa y la LUT de trigonometría. 30 líneas. |
| 3 | **`particles`** | 5 | 2 | ≤500 | Valida estado persistente, `dt`, RNG por efecto y el reciclado de un `Vec`. Es la mejor relación impacto/coste del repo. |
| 4 | **`starfield`** | 5 | 3 | ≤550 | Valida `beat`/`beat_pulse` y la proyección radial con `CELL_ASPECT`. Es el mejor fondo para combinar. |
| 5 | **`matrix`** | 5 | 3 | ≤4 800 (con cap) | Valida los **caps de coste** y el glifo por hash. Sin cap son 36 000 celdas: es la prueba de fuego del presupuesto. |
| 6 | **`flame`** | 4 | 4 | ≤2 400 | Primer buffer 2D con difusión in-place: el patrón de "campo" que luego reutilizan `neural` y `neural-lite`. También valida el hash `(x,y,frame)` en vez de RNG por celda. |
| 7 | **`circle`** | 2 | 1 | ≤360 | Trivial, y es la base geométrica de `pulsar` (ángulos, radio, aspecto). Se hace en 30 minutos y desbloquea el 8. |
| 8 | **`pulsar`** | 5 | 5 | ≤13 000 (con cap) | El más vistoso de todos. Reutiliza `circle` (anillos) y `particles` (orbitales) y añade pozos gravitatorios. Necesita la LUT de seno y el cap de anillos ya probados. |
| 9 | **`neural`** | 4 | 5 | ≤11 400 | Reutiliza `flame` (campos con doble buffer) y `starfield` (partículas/pensamientos). Es el que más paga el slab de neuronas: hacerlo mal (HashMap) arruina el frame. |
| 10 | **`neural-lite`** | 3 | 2 | ≤1 300 | No es trabajo nuevo si el 9 está bien: es el mismo efecto con `ds=2`, difusión submuestreada y cuota de 300 puntos. Y **como modo de calidad baja del 9** es lo que permite bajar el efecto a rects pequeños. Añadir las neuronas tipadas (la mejor idea del fichero) es 100 líneas. |
| 11 | **`fractal`** | 5 | 5 | 120 000 iteraciones | El más caro y el que necesita el presupuesto dinámico (`max_iter` por área) que solo tiene sentido cuando el resto está medido y hay un `FrameBudget` real. Con cache por epsilon se hace asumible. |
| 12 | **`stick`** | 1 | 1 | ≤80 | Barato, pero conceptualmente el peor encajado (borra su rect, glifos incoherentes, no usa bandas). No aporta mecanismo nuevo: se hace al final o no se hace. |
| — | **`tuner`** | — | 2 | ≤10 | **No portar.** No es un visualizador y las 64 bandas no tienen resolución para medir cents (§3.13). Si alguna vez se quiere, necesita el espectro crudo + interpolación parabólica. |

**Camino corto** si hay que entregar algo visible en un día: 1 → 2 → 3 → 4 y el
compositor con `Normal`/`Screen`/`Add`. Eso ya da `aurora` y `clean`, que son dos
presets completos y suficientes para una demo.

**Camino completo**: hasta el 8 se tiene el 90 % del valor visual del repo. El 9
y el 11 son los que dan "wow" y los que cuestan.

**Dependencias entre efectos** (no repetir mecanismo):

```
bars ────────────────┐
wave ────────────────┤
circle ──► pulsar ───┤
particles ───────────┼──► compositor (4.1-4.6)
flame ──► neural ──► neural-lite
starfield ───────────┘
matrix (cap)  ──► fractal (presupuesto por área)
```

---

## 6. Qué copiar de spotatui (y qué no)

Revisado: `README.md`, `docs/themes.md`, `docs/keybindings.md`,
`docs/configuration.md`, `src/core/cover_theme.rs` (488 líneas) y
`src/core/visualizer_look.rs` (852 líneas). spotatui es un fork de
`spotify-tui` convertido en reproductor completo con 7 fuentes, app de escritorio,
plugins Lua, MCP y AI DJ. Es el **anti-modelo de alcance** y a la vez una
referencia muy buena en dos cosas concretas: tema derivado de la portada y look
del visualizador.

### 6.1 Copiar

**1. Portada → tema, pero solo los acentos, con fade y con estado.**
`derive_theme()` (`cover_theme.rs` L186-206) reemplaza **solo** `active`,
`banner`, `hovered`, `selected`, `playbar_progress`, `highlighted_lyrics` y deja
intactos `text`, `background`, `border` y `error`. El comentario del fichero lo
dice: *"legibility never depends on what the art happens to be"*. spote ya deriva
la paleta entera de la portada (más agresivo, y está bien), pero lo que merece
copiarse es el **estado de la transición**:

```rust
enum CoverThemeState {
    Inactive,
    Active   { base: Palette },   // se recuerda el tema propio para volver
    Restoring{ base: Palette },   // fade de vuelta; al acabar → Inactive
}
struct ThemeTransition { from: Palette, to: Palette, progress: f32 }
// progress += elapsed / 0.8s; eased = ease_in_out_cubic(progress); lerp(from,to,eased)
```

Tres detalles que spote no tiene todavía:

- El fade es de **800 ms** y se mide con `elapsed` real, no con ticks
  (`advance(elapsed: Duration)`), así que no depende del framerate.
- `ease_in_out_cubic`, no lineal: el lineal se percibe como un salto al 50 %.
- El tema base se **guarda** durante el fade para poder restaurarlo si se apaga
  el modo "portada → tema" a mitad de transición.

**2. El clamp de acento y la variante de contraste.**
`accent_color()` (L219-226): `v = v.max(0.55)` sobre fondo oscuro, `v.min(0.55)`
sobre claro — un suelo de brillo, no un cambio de saturación. Y
`contrast_variant()` (L228-250) resuelve el caso "dos acentos distintos acaban
iguales tras el clamp": mueve `v` en ±0.225 **sin salirse del rango legal**.
`spote::palette::legible()` hace lo primero; le falta lo segundo (una portada
monocroma puede dar `primary == secondary` y la UI pierde el hover).

**3. Los stops del visualizador derivados de la portada, en OKLab, ordenados por
luminancia.** `VisualizerLook` (`visualizer_look.rs` L13-44):

| Clave | Default | Por qué importa |
|---|---|---|
| `count` | 5 | 5 colores bastan; más se emborrona |
| `order` | `luminance` | **"dark bass to light treble"** — el grave abajo/oscuras, el agudo arriba/claras. Es exactamente el criterio de nuestra `ramp` |
| `min_distance` | 0.08 | distancia mínima en **OKLab** entre dos stops |
| `min_share` | 0.01 | un color que no llega al 1 % de la portada no entra |
| `luminance_floor` | 0.22 | sube los colores oscuros para que las barras se vean |
| `luminance_ceiling` | 0.35 | impide que el blanco del texto pierda legibilidad |
| `fade_ms` | 800 | mismo fade que el tema |
| `greys` | false | descarta negros/blancos/grises si la portada tiene color |

Esto valida el diseño de §2 y da un número concreto: **suelo 0.22 / techo 0.35
para un visualizador que va detrás de texto**. Nuestra `ramp` ya cubre el rango
0..1; para el visualizador detrás del arte conviene quedarse en `0.10..0.60`.

**4. El look de las barras, que es la mejor parte.** `SpectrumLook` (L72-95):

- `peak_hold: 0.35 s`, `peak_gravity: 2.4` (alturas/s²), `fall_per_second: 3`
  (una barra cae a 3 alturas completas por segundo). Esto es lo que hace que unas
  barras parezcan un VU de verdad en vez de un ecualizador nervioso.
- `pulse_alpha: 0.35`, `pulse_height: 0.5`, `pulse_bands: 0.125`: el 12.5 % de
  las bandas (los graves) modula **media altura** de todas las barras. Es la
  máscara `Band` de §4.4, con nombres y números.
- `cap_height: 2`, `cap_gap: 2`, `glow_side: 2`, `glow_end: 3`,
  `glow_alpha: 0.12`, `foot_alpha: 0.28`, `top_alpha: 0.08`: el degradado de la
  barra es más opaco abajo (0.28) que arriba (0.08) y tiene un halo lateral. Con
  la rampa del álbum, traducir a `ramp_at(0.3 + 0.65*(j/h))` y `alpha` por fila.
- `waveform.lines: [[-0.6, 1, 0.18], [1, 9, 0.06], [1, 4, 0.16], [1, 1.6, 0.85]]`:
  la onda se dibuja **4 veces** — un reflejo desplazado al 18 %, dos halos anchos
  y la línea nítida al 85 %. Es un truco de 8 líneas que convierte `wave` de
  "una línea de puntos" en algo que parece brillar. **Merece copiarse tal cual**
  en `wave`.

**5. Patrón de parseo de configuración.** `visualizer_look.rs` L225-330: cada
clave es opcional, un tipo equivocado **mantiene el default y emite un warning
con la ruta de la clave**, un número fuera de rango se **clampea** con warning, y
una clave desconocida avisa con su path. El fichero se relee en menos de un
segundo tras cada guardado (y detecta guardados a medias: si el segundo intento
falla igual, deja de intentarlo). Para el `config.toml` de spote: copiar el
patrón entero (warning con path + clamp + relectura), no solo el formato.

**6. UX / keybindings concretos que valen la pena**

- `?` abre la ayuda con **buscador** (`/` filtra por tecla, descripción o
  contexto). En una TUI con 40 teclas esto no es un lujo.
- El mismo buscador en **Settings**, con match **fuzzy** (los caracteres aparecen
  en orden: `volinc` → Volume Increment) y ordenado por relevancia. Se aplica al
  tab actual y **sobrevive** al cambio de tab.
- `v` abre el visualizador, `V` cicla el estilo — nuestra §7.3 ya tiene `v`/`V`
  para eso. Coincide.
- Vistas que se **desacoplan a un panel** (`Alt+d`): letras, visualizador,
  portada, cola y miniplayer pueden convivir en el layout como hojas de un árbol
  (`main, lyrics, visualizer, cover_art, queue, mini_player`, cada hoja una sola
  vez). Es la misma idea que "el viz detrás del arte o en una franja".
- **No duplicar**: el playbar esconde su miniatura de portada cuando el panel
  Now Playing está abierto. Regla general: un dato, un sitio.
- Nombrar los looks en vez de numerarlos: `Cava`, `BarGraph`, `HeadUnit`,
  `AnalogVu`, `Winamp`, `PhosphorScope`, `Outrun`, `RackEq`, `Cassette`. Nuestros
  presets deberían tener nombre (`aurora`, `inferno`, …) y mostrarlo 1.5 s al
  cambiar.

### 6.2 No copiar

| Idea de spotatui | Por qué no |
|---|---|
| **Streaming de Spotify con cliente propio** (librespot fork, pairing OAuth, Premium) | Es el 80 % del repo y otro producto. spote habla MPRIS con lo que ya suena. Meter un cliente de streaming cambia el proyecto de "TUI de música" a "reproductor". |
| 7 fuentes (Local, Subsonic/Navidrome, Radio, YouTube, Qobuz, Apple Music) | Cada una es un backend, un modelo de datos y un conjunto de bugs. spote no las necesita para ser bueno. |
| AI DJ, servidor MCP, plugins Lua, listening party, friends, `desktop/`, `gui/`, `mobile/`, `agent-plugin/`, telemetría, self-update, Discord RPC | Cada uno es una funcionalidad de un producto distinto. Ninguno mejora la visualización. |
| 14 presets de tema + 16 slots de color con nombre + `preset: "Custom"` | **Compite con la premisa de spote**: la portada elige el color. Un sistema de presets manuales invita a fijar colores y a que la portada deje de mandar. Como mucho: un modo `neutral` (sin portada) y un `mono`. |
| `cover_art_dither_algorithm: stucki / bayer8x8 / atkinson` + `pixel_scale` | spote ya tiene `Dither` (Bayer 8×8), `Half`, `Braille` y `Off`. Añadir kernels de dithering es trabajo de imagen con poquísimo retorno en una TUI. |
| `cover_art_dither_color: auto` que sigue el color del playbar | Acopla el arte al color del progreso. Un acoplamiento que hay que recordar cada vez que se toque cualquiera de los dos. |
| 40 acciones rebindables + `config.yml` reescrito desde la pantalla de ajustes | Correcto para ellos, prematuro para spote: primero que las teclas sean buenas, luego que sean configurables. |
| CHANGELOG de 262 KB, `.all-contributorsrc`, wiki submodule | Infraestructura de proyecto de 200 contribuidores. |

**Resumen en una línea**: copiar el *tema derivado de la portada con fade y con
clamp de legibilidad*, el *look de barras con picos y halos*, y el *patrón de
configuración con warnings*. No copiar nada del stack de fuentes ni del ecosistema
alrededor.

---

## 7. Anexos

### Anexo A: glifos y anchos en terminal

**La regla:** una celda de terminal = un glifo de ancho 1. Si escribes un glifo de
ancho 2, ratatui avanza una celda pero el terminal avanza dos: **todo el arte a la
derecha de esa celda se desplaza**. Es el bug visual más difícil de diagnosticar
porque solo aparece con ciertas canciones.

Clases de `East_Asian_Width` (UCD):

| Clase | Ancho | Ejemplos | Veredicto |
|---|---|---|---|
| `Na` (Narrow), `H` (Halfwidth) | 1 | ASCII, Latin-1, `─│┌┐└┘`, katakana *halfwidth* U+FF61-FF9F | **seguro** |
| `N` (Neutral) | 1 | braille U+2800-28FF, símbolos matemáticos | seguro |
| `A` (Ambiguous) | 1 en terminales no-CJK, **2 en modo CJK** | `·` `•` `○` `●` `★` `♦` `░▒▓█▄▀` `╔╗╚╝` `‖` `°` `×` `±` | aceptable con reservas |
| `W` (Wide), `F` (Fullwidth) | 2 | CJK, katakana U+30A0-30FF, hangul, **`⚡` U+26A1** | **prohibido** |

Extra: los caracteres con `Emoji_Presentation=Yes` (o seguidos de U+FE0F) se
renderizan como emoji de 2 celdas y en color, ignorando tu `fg`. Prohibidos
siempre.

**Glifos de cli-viz, uno a uno:**

| Efecto | Set original | Veredicto |
|---|---|---|
| `bars` | `█` | A (aceptable; universal en la práctica) |
| `wave` | `•` | A |
| `circle` | `•` `*` `★` | `★` es A: en fuente CJK rompe. Cambiar a `●` (A) o `o` (Na) |
| `particles` | `.` `*` `+` `•` `○` `◌` `◦` | `◌` `◦` son A (rango 25xx). Dejar `.` `*` `+` `•` `○` |
| `flame` | `' .,:;=+*#%@'` + `▄` | ASCII + `▄` (A). Correcto |
| `matrix` | ASCII 33-126 + **U+30A0-30FE** | **`W`: prohibido.** Usar ASCII + katakana halfwidth U+FF61-FF9F (clase H, ancho 1) |
| `starfield` | `.` `*` `+` `·` `-` `|` `=` `‖` `°` | `‖` (A, y en algunas fuentes 2) y `°` (A): cambiar por `|`, `o` |
| `fractal` | 70 glifos: ASCII + `'` `` ` `` `^` | todo ASCII. **El mejor set del repo**: densidad real (0..9 distintos niveles de tinta) |
| `neural` | `·•○◙★✧♦❂☼❈♠♣⚡✿❀⚘❄✺⚕` | **`⚡` es `W` → prohibido.** El resto son A y muchos con `Emoji=Yes` (`❄` `⚘` `⚕`): sustituir el set entero |
| `neural-lite` | `·•○★✧♦◆❅✺` | `❅` `✺` son A/Emoji: sustituir |
| `pulsar` | `·∘○◎●` `*` `.` `#` `'O'` | `◎` `●` son A. Aceptable; `∘` casi nunca está en la fuente |
| `stick` | `O` `|` `/` `\` `_` | ASCII puro. Correcto |
| `tuner` | `┌─┐│└┘╌ ┼ ┴ █ ▼ •• 《 》` | `《》` son `W` → prohibidos; `╌` `┼` `┴` son A (box drawing), aceptables |

**Sets recomendados** (todos verificables como ancho 1, en orden de densidad):

```text
DENSITY_ASCII : " .:-=+*#%@"                        // 10 niveles, universal
DENSITY_RICH  : " .,:;'-~=+*oO0#%@&"                // 17 niveles, todo ASCII
BLOCKS        : " ▁▂▃▄▅▆▇█"                        // A, para barras de 1 fila
BRAILLE       : U+2800..U+28FF                      // 2x4 puntos, clase N, ancho 1
BOX           : "─│┌┐└┘├┤┬┴┼"                       // A
KATAKANA_HW   : U+FF66..U+FF9D                      // clase H, ancho 1 (matrix)
```

**Verificación obligatoria** (si no, el bug aparece en producción con la portada
de alguien):

```rust
use unicode_width::UnicodeWidthChar;

#[cfg(debug_assertions)]
fn assert_width_one() {
    for &ch in GLYPHS { assert_eq!(ch.width(), Some(1), "glifo de ancho != 1: {ch:?} U+{:04X}", ch as u32); }
}
```

Y en release, un `debug_assert!` en `LayerBuf::set` es suficiente. La crate
`unicode-width` ya viene con ratatui, no es dependencia nueva.

Ojo con eso: en el código actual (`viz/effects.rs`) el set `DOTS` es
`['·', '•', '∙', '◦', '○', '●']`, y **los seis son Ambiguous**. Con la aserción
estricta (`width == 1`) fallarían todos; con fuente CJK se pintan a 2 celdas.
Dos salidas, elegir una y documentarla: (a) aceptar `A` como legal y añadir un
modo `ascii_only` que sustituya el set por `. * + o O #`; o (b) quedarse en ASCII
y no tener el problema. `BLOCKS` y `SHADES` (bloques y sombras) están en el mismo
caso.

**Glifos de bloque y `bg`.** `█` `▄` `▀` son opacos: pintan con `fg` y el `bg` de
la celda no se ve. Un efecto que los use **delante** del arte está tapando el
arte; detrás, no se ve el efecto. Es la razón de que `bars` vaya con `Add` y no
`Normal` en `inferno`.

### Anexo B: checklist de port

Antes de dar un efecto por terminado:

- [ ] `resize(w, h)` reserva los buffers y **no** se llama por frame.
- [ ] `render()` no hace ni una alloc (comprobar con `#[global_allocator]` contando
      allocs: debe ser 0 por frame).
- [ ] No hay `format!`, `String`, `Vec::new`, `Box::new`, `to_string`,
      `collect::<Vec<_>>` en la ruta de render.
- [ ] Todo el tiempo va por `ctx.dt` y `ctx.time`; no hay `Instant::now()` dentro.
- [ ] El RNG es un campo del efecto; ≤64 llamadas/frame (o hashes).
- [ ] Todas las escrituras pasan por `LayerBuf::set`, que recorta.
- [ ] Todos los glifos están en `GLYPHS` y pasan el test de ancho 1.
- [ ] El peor caso de coste está acotado por un `cap` explícito (drops, anillos,
      iteraciones, partículas).
- [ ] Con `rect` de 20×6 no peta y no dibuja basura (probar la degradación).
- [ ] Con `rect` de 240×70 el frame sigue <8 ms (probar el caso grande).
- [ ] Con `bands` todas a cero (silencio) no hay NaN ni división por cero:
      `mean()` de un rango vacío, `dist_sq = 0`, `log(0)`, `sin(inf)`.
- [ ] Al cambiar de pista (paleta nueva) no hay salto: la rampa se interpola
      (§6.1) y el efecto no cachea colores RGB entre frames.
- [ ] En pausa, el efecto se congela (no avanza `time` ni `frame`) pero sigue
      visible.
- [ ] Detrás del arte: `opacity ≤ 0.55`, sin `BOLD`, sin `bg`, techo de rampa
      §2.4.
- [ ] Delante del arte: contraste ≥ 2.0 contra el `fg` del arte local (§4.6 C).
- [ ] El nombre del efecto y sus parámetros están en el panel de ajustes, no en el
      teclado global.

**Pruebas de audio mínimas** (el equivalente a los tests del repo, que no tiene
ninguno):

1. Silencio → nada se mueve, nada peta.
2. Tono de 60 Hz a −6 dBFS → `bass ≈ 0.8`, `mid ≈ 0.1`: la llama sube, las barras
   de la izquierda al máximo, las estrellas nacen con `'*'`.
3. Tono de 1 kHz → `mid ≈ 0.8`: el fractal sube iteraciones, el pulsar acelera el
   muestreo de los anillos.
4. Ruido blanco → todo a la vez, es el test de estrés del compositor.
5. Un `beat` sintético cada 500 ms → `particles`, `pulsar`, `starfield` y
   `matrix` deben mostrar algo distinto **en el frame del beat**, no dos frames
   después.
