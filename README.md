# spote

Reproductor de música en terminal. La portada del disco tiñe la interfaz y los
bordes, el playhead y las letras reaccionan al audio. La TUI está en Rust y tira
de `mpv` para reproducir, `yt-dlp` para bajar y SQLite para la biblioteca.

![spote](docs/screenshots/now.png)

## Instalar

```bash
git clone https://github.com/fabiomode2/spote && cd spote
./scripts/install.sh
spote
```

Hacen falta Rust 1.82+ para la TUI (1.85+ si compilas `spote-mini`), `mpv`, `ffmpeg` y `yt-dlp` en el `PATH`. El instalador
deja los binarios en `~/.local/bin`, crea el venv con `faster-whisper` y copia
los scripts. Las letras no necesitan nada más: LRCLIB, Genius, captions de
YouTube y ASR van dentro.

Para actualizar: `./scripts/update.sh`, o desde el aviso de versión que sale en
la propia TUI.

## Qué hace

- **Arte**: portada en half-block, dither, braille o imagen real en Kitty. La
  paleta del tema sale de la carátula.
- **Visuales**: `clean`, `aurora`, `bloom`, `inferno`, `crt`, `lcd`, `rack`,
  `braille` y `stage`; `v` cicla y `V` los pone a pantalla completa.
- **Letras**: Genius es la referencia de texto. LRCLIB, lrcmux y las captions
  de YouTube solo aportan tiempos cuando su letra coincide; las captions no
  generan texto y el ASR solo alinea una letra existente. Si no hay una fuente
  fiable, se muestra “sin letra” antes que inventarla. La barra de la letra
  permite zoom (`A−`/`A+`), ajuste fino de ±100/±250 ms por pista, scroll
  manual con rueda o flechas y volver a la línea actual. `⟳` descarta la caché
  y busca otra letra si la que había no era la canción.
- **Karaoke**: palabra a palabra, frases largas en dos filas y la última palabra
  se llena antes de cambiar de línea.
- **Fondo mosaico**: la portada de la pista actual repetida en mosaico detrás
  de toda la interfaz, con opacidad de 5–50 %, movimiento en deriva, filas,
  columnas o giro, velocidad ajustable y saltos al compás del bombo.
- **Búsqueda y reproducción**: Spotify y Deezer como catálogo, sin cuenta. El
  audio sale de `yt-dlp`. Si una pista no está descargada, se resuelve una URL
  y suena al momento mientras se guarda la copia local. Los títulos simbólicos
  (`(_8(|)`) conservan su núcleo para no confundirlos con homónimos. `I` importa
  playlists y álbumes.
- **Cola y listas**: encolar, reordenar, descargar, favoritas y playlists
  locales. `z` alterna tarjetas con portada y lista compacta. `/` en la
  biblioteca abre un filtro inline por título, artista o álbum.
- **Media del sistema**: MPRIS bidireccional. spote aparece como reproductor
  en el escritorio (teclas multimedia, OSD, widgets) y `t` sigue lo que suene
  en Spotify, SoundCloud, VLC o cualquier otro reproductor MPRIS.
- **Mini reproductor**: `w` abre `spote-mini`, una ventana pequeña siempre
  encima con portada, espectro, progreso y controles. También se lanza con
  `spote mini`.
- **Audio ajustable**: pestaña `♫ audio` con ecualizador gráfico de 10 bandas,
  preamp, normalizador en directo y limitador. La nivelación entre pistas mide
  el audio completo y deja todas las canciones al mismo volumen; se aplica en
  caliente y se puede apagar desde la propia pestaña o desde ajustes.
- **Salas**: `spoted` añade presencia, chat y cola compartida.
- **Layout**: `E` edita paneles, anchos, dock y tamaños. Se guarda solo en
  `config.toml`.

## Atajos

| | | | |
|---|---|---|---|
| `space` | play/pausa | `a` | encolar |
| `n` / `p` | siguiente / anterior | `f` | favorita |
| `←` `→` | seek ±5 s | `g` | artista |
| `/` | buscar · filtro en biblioteca | `T` | recortar intro/outro |
| `u` | bucle de la pista | `F` | portada a pantalla completa |
| `S` | aleatorio on/off | `o` | bucle: off / todo / una |
| `c` / `C` | portada / letra | `V` | visualizador a pantalla completa |
| `E` | editor de diseño | `Esc` | limpiar búsqueda |
| `A` | pestaña de audio | `w` | mini reproductor |
| `t` | seguir el sistema (MPRIS) | `?` | ayuda |
| `q` | salir de spote | | |

## Cómo funciona

```
audio local ──▶ mpv (IPC) ──▶ FFT ──▶ UI reactiva
     │                         │
     └──▶ SQLite ◀─────────────┘   librería · playlists · letras · historial
```

Config, datos y caché en XDG (`~/.config/spote`, `~/.local/share/spote`,
`~/.cache/spote`). `SPOTE_HOME` los reubica todos.

```toml
[ui]
art_mode = "default"   # default | half | pixel | braille | off
viz = "clean"          # clean | aurora | bloom | inferno | crt | lcd | rack | braille | stage
fps = 60               # frames de la UI (20-60)
fx_fps = 60            # ritmo de los efectos; a pantalla completa va a la mitad

[ui.fx]
ambient = "mosaic"     # off | void | aurora | neural | glitch | lyrics | cover | mosaic
ambient_opacity = 35   # 0-50 %
mosaic_motion = "drift" # drift | rows | columns | rotate
mosaic_speed = 4       # 1-10
mosaic_beat = true     # salta con el bombo

[player]
streaming = true       # resuelve YouTube y suena mientras descarga
stream_cache = true    # guarda la copia local al terminar (tras unos segundos de escucha)

[playback]
repeat = "off"         # off | all | one
shuffle = false

[audio]
enabled = true
eq_enabled = true
bands = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]   # dB, 31 Hz … 16 kHz
preamp_db = 0
normalizer = "off"     # off | dynamic | loudnorm (respaldo en directo)
normalizer_strength = 0.5
normalize_tracks = true # iguala el volumen de todas las pistas
limiter = false

[lyrics]
asr = true
asr_model = "small"    # tiny | base | small | medium | large-v3
```

## Comandos

```bash
spote ingest ~/Music                            # indexa audio local
spote import <url-spotify> --download           # importa y baja
spote doctor                                    # comprueba dependencias
spote record [--flac]                           # graba lo que suena
spoted                                          # backend social en :7070
```

## Más

- `idea.md` — producto, arquitectura y decisiones.
- `docs/arquitectura.md` — módulos, layout y mapa de audio.
- `docs/effects.md` — especificación de los efectos.
- `docs/rework.md` — estado y limitaciones.
- `docs/screenshots/` — más capturas.
