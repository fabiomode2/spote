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
  de YouTube solo aportan tiempos cuando su letra coincide. Si no hay una
  fuente fiable, se muestra “sin letra verificada”. El chip `inv` (o `G` en el
  foco de letra) permite activar/desactivar las letras generadas con
  captions/ASR; por defecto están apagadas. La barra permite zoom (`A−`/`A+`),
  ajuste de ±100/±250 ms por pista, scroll manual y volver a la línea actual
  (`⟲`/`home`). `⌕`/`b` abre una búsqueda manual: lista las versiones de
  LRCLIB, las previsualiza y deja aceptar a mano la que sea correcta. `⟳`
  descarta la caché y busca otra letra.
- **Karaoke**: palabra a palabra, frases largas en dos filas y la última palabra
  se llena antes de cambiar de línea.
- **Ficha de artista y álbum**: se abren en un panel fijo a la derecha, no en
  un popup. El artista muestra su discografía en una tira horizontal
  scrolleable y sus canciones populares; el álbum, portada, ficha y tracklist.
  Se cierran solo con `‹ volver`/`esc`.
- **Barra de canción actual**: portada, título, artista, álbum, corazón de
  favorita y estado de reproducción, siempre encima del transporte.
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
- **Discos guardados**: pestaña `discos` con todos los álbumes de la
  biblioteca agrupados por artista, portadas y un buscador propio (`/`).
  `X` o ⇧clic en la × oculta una pista de la biblioteca sin borrar el audio
  ni su historial; en favoritas quita el corazón. `d` sobre una lista y `X`
  sobre un disco piden confirmación y quitan **todas sus canciones** de la
  biblioteca (el audio sigue en disco); en la lista, `t` permite conservarlas.
  `spote dedupe` fusiona duplicados de forma explícita, con `--dry-run` y
  backup previo.
- **Wrapped**: `W` abre un resumen de escucha con horas, reproducciones,
  artistas, canciones, discos y actividad por meses. `a` alterna entre el
  año actual y todo el tiempo. Manteniendo `shift` aparece una `×` en cada
  fila para ocultar varias seguidas de un clic.
- **Media del sistema**: MPRIS bidireccional. spote aparece como reproductor
  en el escritorio (teclas multimedia, OSD, widgets) y `t` sigue lo que suene
  en Spotify, SoundCloud, VLC o cualquier otro reproductor MPRIS.
- **Mini reproductor**: `w` abre `spote-mini`, una ventana pequeña siempre
  encima con la estética del transporte de la TUI: tipografía monoespaciada,
  barra de bloques, controles pixel-art y la portada del disco sonando. El
  visual de la ventana grande ocupa todo el fondo y el chip `viz` cambia de
  preset desde el propio mini; también se lanza con `spote mini`.
- **Audio ajustable**: pestaña `♫ audio` con ecualizador gráfico de 10 bandas,
  preamp, normalizador en directo y limitador. La nivelación entre pistas mide
  el audio completo y deja todas las canciones al mismo volumen; se aplica en
  caliente y se puede apagar desde la propia pestaña o desde ajustes. Dentro de
  la pestaña, `←`/`→` cambia de control y `↑`/`↓` ajusta el valor.
- **Salas**: `spoted` añade presencia, chat y cola compartida. La API exige una
  clave de cliente por persona (`X-Spote-Key`), las sesiones están atadas a esa
  clave y caducan. Ver [`docs/social.md`](docs/social.md).
- **Cuenta y sync**: la misma clave en PC y Android identifica la misma cuenta;
  `spoted` sincroniza favoritas, escuchas y playlists por revisiones, sin subir
  audio ni rutas locales.
- **Layout**: `E` edita paneles, anchos, dock y tamaños. Se guarda solo en
  `config.toml`.

Los vídeos con verificación de edad necesitan una cuenta de YouTube ya
verificada. Si configuras `[stream] cookies_from_browser`, spote reintenta el
streaming y la descarga con las cookies de ese navegador. Para este tipo de
vídeos también hace falta tener la edad verificada en la cuenta; sin eso
YouTube no entrega ningún formato.

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
| `X` | quitar de la biblioteca | `t` | seguir el sistema (MPRIS) |
| `q` | salir de spote | `?` | ayuda |

## Cómo funciona

```
audio local ──▶ mpv (IPC) ──▶ FFT ──▶ UI reactiva
     │                         │
     └──▶ SQLite ◀─────────────┘   librería · playlists · letras · historial
```

Config, datos y caché en XDG (`~/.config/spote`, `~/.local/share/spote`,
`~/.cache/spote`). `SPOTE_HOME` los reubica todos. Para el VPS hay
`Dockerfile`, `docker-compose.yml` y `deploy/nginx/spote.conf`; el paso a paso
está en [`docs/social.md`](docs/social.md).

```toml
[server]
url = "https://spote.example.com"  # vacío = modo local, sin salas
name = "ana"
invite = "…"           # la invitación del grupo
client_key = "…"       # clave personal; sin ella la API responde 401
token = ""             # se rellena tras el login

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

[stream]
cookies_from_browser = ""  # firefox | chrome | brave | chromium | edge
cookies_file = ""          # cookies.txt exportado; tiene prioridad si no está vacío

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
allow_generated = false # captions/ASR pueden inventar letra (chip inv)
asr_model = "small"    # tiny | base | small | medium | large-v3
```

## Comandos

```bash
spote ingest ~/Music                            # indexa audio local
spote import <url-spotify> --download           # importa y baja
spote dedupe --dry-run                          # lista duplicados sin tocar nada
spote dedupe --yes                              # fusiona con backup previo
spote doctor                                    # comprueba dependencias
spote record [--flac]                           # graba lo que suena
spoted                                          # backend social (requiere secretos)
spoted keygen ana bea                           # genera invitación y claves
docker compose up -d --build                    # spoted + SQLite en un volumen
```

## Más

- `idea.md` — producto, arquitectura y decisiones.
- `docs/social.md` — backend `spoted`, seguridad, Docker y nginx.
- `docs/arquitectura.md` — módulos, layout y mapa de audio.
- `docs/effects.md` — especificación de los efectos.
- `docs/rework.md` — estado y limitaciones.
- `docs/screenshots/` — más capturas.
