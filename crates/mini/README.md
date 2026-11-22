# spote-mini

Mini reproductor de escritorio (420×220, sin decoraciones, siempre encima) que
pinta el estado que le manda la app principal y le devuelve comandos. La ventana
es `egui`/`eframe`; el transporte es un socket Unix local con mensajes JSON por
línea.

El crate solo implementa **el cliente y la UI**. El servidor lo pone la app
principal (o el componente que corresponda); no hay ninguna implementación de
servidor aquí.

## Lanzar

```bash
cargo build -p spote-mini
./target/debug/spote-mini

# o directamente
cargo run -p spote-mini
```

No acepta argumentos ni variables propias. Si el socket no existe o nadie
escucha, la ventana queda abierta mostrando `spote no está abierto` y reintenta
la conexión cada segundo. Al cerrar con el botón de la cabecera, o cuando el
servidor manda `Quit`, el proceso termina limpiamente y avisa al servidor con
`Quit`.

## Socket

La ruta la define una única función pública del protocolo:

```rust
spote_protocol::mini::mini_socket_path()
```

Precedencia exacta:

1. `$XDG_RUNTIME_DIR/spote-mini.sock` si `$XDG_RUNTIME_DIR` existe como
   directorio.
2. `$SPOTE_HOME/cache/spote-mini.sock` si `$SPOTE_HOME` está definido.
3. `$XDG_CACHE_HOME/spote/spote-mini.sock` si `$XDG_CACHE_HOME` está definido.
4. `/tmp/spote-mini-$UID.sock`.

El servidor debe llamar a esa misma función y crear el directorio padre antes
de `bind`. El cliente no crea directorios. Si el servidor deja un socket viejo,
es responsabilidad suya retirarlo antes de escuchar.

## Formato de línea

NDJSON: un valor JSON por línea, UTF-8, terminado en `\n`. El cliente tolera
`\r\n` y espacios alrededor. No hay cabecera, longitud ni handshake.

### Servidor → cliente: `MiniSnapshot`

```json
{"title":"Hyperballad","artist":"Björk","album":"Post","playing":true,"position_ms":42500,"duration_ms":326000,"volume":0.8,"shuffle":true,"repeat":"all","art_path":"/home/manu/.cache/spote/art/post.jpg","accent":[32,180,164],"bands":[0.1,0.4,0.9],"status":"playing","track_key":"isrc:GBBKS9500001"}
```

| Campo | Tipo | Significado |
|---|---|---|
| `title` | string | título de la pista |
| `artist` | string | artista |
| `album` | string \| null | álbum; `null` si no hay |
| `playing` | bool | reproduciendo o en pausa |
| `position_ms` | u64 | posición en milisegundos |
| `duration_ms` | u64 | duración; `0` si no se conoce |
| `volume` | f32 | `0.0..=1.0` (la UI lo enseña como 0–100 %) |
| `shuffle` | bool | aleatorio activo |
| `repeat` | string | `off`, `all` o `one` |
| `art_path` | string \| null | ruta local de la portada |
| `accent` | `[u8; 3]` | RGB dominante de la portada |
| `bands` | array f32 | espectro de graves a agudos, idealmente 32 valores `0.0..=1.0` |
| `status` | string | texto libre (`playing`, `paused`, `buffering`…); se ve en el tooltip del indicador |
| `track_key` | string | identidad de la pista; al cambiar se reinicia el espectro suavizado |

Los campos ausentes toman su valor por defecto (`0`, `false`, `null`, listas
vacías). El servidor puede mandar el snapshot completo cada vez que cambie algo
y, además, con la frecuencia que quiera para refrescar `position_ms`. La UI
extrapola la posición mientras `playing` es `true`, así que no hace falta un
snapshot por frame.

Si `bands` viene vacío, la UI dibuja una animación discreta. Si trae menos de 32
valores, los interpola; si trae más, usa los 32 primeros. Los valores no finitos
se tratan como `0.0`.

Si `art_path` no se puede abrir, se pinta un vinilo con `accent`. La ruta puede
ser absoluta o relativa al directorio de trabajo del proceso, pero lo normal es
absoluta.

### Cliente → servidor: `MiniCommand`

Variantes unitarias (JSON string):

```json
"PlayPause"
"Next"
"Previous"
"Shuffle"
"CycleRepeat"
"Quit"
```

Variantes con dato (JSON objeto, tagged externamente por serde):

```json
{"Seek":42500}
{"SetVolume":0.8}
```

`Seek` va en milisegundos. `SetVolume` va en `0.0..=1.0`; el cliente recorta
antes de enviar. `CycleRepeat` pide al servidor que avance `off → all → one →
off`; la UI no cambia el estado por su cuenta. Al recibir `"Quit"` del servidor,
el cliente cierra la ventana. Al cerrar la ventana, el cliente manda `"Quit"`
(best effort) y termina.

Ejemplo mínimo de servidor (esbozo, no incluido):

```rust,ignore
let path = spote_protocol::mini::mini_socket_path();
let listener = std::os::unix::net::UnixListener::bind(path)?;
for stream in listener.incoming() {
    let mut writer = stream?;
    // Mandar snapshots:
    writer.write_all(snapshot.to_line()?.as_bytes())?;
    // Leer comandos con BufReader::lines():
    //   MiniCommand::from_line(&line?)
}
```

## UI y comportamiento

- 420×220 puntos, no redimensionable, sin decoraciones, `always_on_top`.
- Cabecera arrastrable con `ViewportCommand::StartDrag`; título, artista/álbum,
  indicador de conexión y botón de cerrar.
- Portada local con textura `egui`; si falta, vinilo dibujado con `accent`.
- Espectro de 32 barras con ataque/decaimiento suavizados, picos y `accent`.
- Progreso clicable y arrastrable (seek con throttle de 100 ms y envío final al
  soltar); tiempos `m:ss` o `h:mm:ss`.
- Controles dibujados con triángulos y barras: shuffle, anterior, play/pausa,
  siguiente y repeat (`1` dibujado cuando toca). Estados activos en `accent`.
- Volumen con slider 0–100 %, icono de altavoz dibujado y actualización
  optimista durante el arrastre.
- Dos hilos fuera de la UI: el lector conecta, lee snapshots, guarda el último y
  avisa de los cambios de conexión; el escritor manda los comandos por un clon
  del mismo socket. La UI solo hace `try_recv`, `take_snapshot` y `send`. Si no
  hay conexión, se descartan comandos viejos y se reintenta cada 1 s.

## Decisión de versión y features

`eframe 0.32.3` (versión estable fijada dentro de la línea 0.32.x), con:

```toml
default-features = false
features = ["glow", "x11", "wayland", "default_fonts"]
```

- `glow` usa OpenGL y evita el stack wgpu/Vulkan, que aquí no hace falta y
  añade peso de compilación y dependencias.
- `x11` y `wayland` cubren las dos sesiones de este equipo.
- `default_fonts` evita depender de fuentes del sistema.

Comprobado en este equipo: `pkg-config` encuentra `x11 1.8.12`,
`wayland-client 1.23.1`, `xkbcommon 1.13.1`, `gl 1.2` y `fontconfig 2.15.0`;
`libGL`, `libEGL`, `libwayland-client` y `libxkbcommon-x11.so.0` están
instalados. `xkbcommon-x11.pc` no existe, pero winit/x11rb carga la librería en
runtime y la compilación no la necesita. No faltó ninguna librería de sistema.
Si en otra máquina falta algo, en Debian/Ubuntu suele bastar con
`libx11-dev libxkbcommon-dev libgl1-mesa-dev libfontconfig1-dev libwayland-dev`.

## Validación

```bash
CARGO_TARGET_DIR=/tmp/opencode/target-mini cargo check -p spote-mini
CARGO_TARGET_DIR=/tmp/opencode/target-mini cargo test -p spote-mini
CARGO_TARGET_DIR=/tmp/opencode/target-mini cargo test -p spote-protocol mini
```

`cargo test -p spote-mini` levanta un `UnixListener` temporal y comprueba el
roundtrip real: conexión, snapshot, comando y `Quit` del servidor. No abre
ninguna ventana.

No se lanzó una ventana gráfica interactiva; la validación es de compilación y
tests.
