# spote — Spotify privado, con TUI de la hostia

> Documento de verdad del proyecto. Si eres un agente y acabas de aterrizar aquí:
> lee esto entero antes de escribir una línea. Al final hay una sección de
> convenciones y otra de estado real del repo.

Última actualización: 2026-11-13.

---

## 1. Qué es

Un reproductor de música **privado para 4 amigos**, con catálogo abierto
(YouTube / YT Music / SoundCloud / ficheros locales / audio extraído de Spotify)
y una capa social propia: presencia, salas de escucha sincronizada, cola
compartida, historial y letras.

Tres caras, un solo núcleo:

```
core local (catálogo + descarga + reproducción)
   ├── spote        TUI (la cara principal, la que tiene que ser memorable)
   ├── spote web    (fase posterior, misma API local + API social)
   └── Android      (fase posterior, protocolo congelado primero)
spoted              backend social en VPS (sin audio, sin yt-dlp)
```

La frase que resume el producto: **"busco cualquier cosa, suena en 2 segundos,
y mis colegas ven lo que estoy oyendo / escuchan lo mismo conmigo al mismo
tiempo"**.

Lo que **no** es:

- No es un clon de Spotify con sus 40 pantallas. Cuatro pantallas bien hechas.
- No es un servidor de medios. El VPS nunca ve un byte de audio.
- No es un scraper de Spotify. Spotify es catálogo/importador, nunca fuente
  (con un matiz: ver §6, el audio extraído del cliente oficial entra por
  `ingest` como cualquier otro fichero local).

---

## 2. De dónde viene esta idea (historial)

Dos sesiones previas, ambas con decisiones tomadas. No las repitas, ya están
cerradas:

1. **Sesión de opencode** `ses_e389cd185ffetmopQLTaWSe9uz`, "Cliente musical
   para Android y PC" (10 nov 2026, en
   `~/.local/share/opencode/opencode.db`). Aquí nace todo: yt-dlp, VPS social,
   autoridad del tiempo en el servidor, clave canónica de track, handshake de
   `ready`, pitch-corrected drift correction, spotatui como referencia.
2. **Sesión de pi** en `~/.pi/agent/sessions/--home-fabio-Code-spote--/`
   (12 nov 2026). Se investigó en vivo el cliente oficial de Spotify: qué se
   puede sacar y qué no. Resultados en §6.

Decisiones ya tomadas (no reabrir sin motivo nuevo):

- Cada cliente descarga y reproduce **local**. El VPS solo coordina.
- `yt-dlp` se llama como **binario externo** (subprocess), nunca como librería.
- Reproducción mientras descarga: servidor/fichero creciente con `Range`, no
  preview de 10 s. (En local es simplemente: mpv abre el `.part` en cuanto hay
  bytes y el downloader sigue escribiendo; ver §5.3.)
- `mpv` es el motor de audio, por IPC. No se reimplementa un player.
- Salas: el servidor es autoridad del **tiempo**, no del stream. Cada cliente
  reproduce su propio fichero.
- Clave canónica de canción independiente del proveedor (`isrc:` / `sha1:`).
- Tolerancia de duración **±2 s** para aceptar un match en salas (±4 s al
  importar). Un cover es peor que un fallo.
- Proveedor preferente: audio de YT Music (canal `- Topic`), luego YouTube,
  luego SoundCloud.
- Mantenimiento de yt-dlp es permanente (rompe cada pocas semanas). Actualización
  desde la propia app (`spote doctor`).

---

## 3. Stack técnico (decidido)

**Rust en todo el workspace.** Un `Cargo.toml` con crates:

```
crates/
  protocol/   tipos compartidos: TrackRef, RoomState, mensajes WS, DTOs
  core/       todo lo que no es UI
              config · library (SQLite+FTS5) · downloader (yt-dlp)
              player (mpv IPC) · lyrics (LRCLIB/LRC) · artwork (paleta+ascii)
              spectrum (ffmpeg→FFT) · spotify (MPRIS) · clock/sync
  server/     spoted: axum + SQLite + WebSocket (capa social)
  tui/        spote: ratatui + crossterm (la joya)
```

Por qué Rust y no Python/Textual (que es lo que hay en lyricbar):

- 60 fps con un buffer de células RGB + 4 capas de efectos compuestas. En
  Python eso son 15-25 fps con suerte, y se nota.
- Arranque instantáneo, binario único, sin venv. La TUI se abre en <50 ms.
- El mismo lenguaje para el protocolo y el servidor: los tipos de la sala son
  los mismos en los tres sitios, el compilador evita desincronizaciones.
- `ratatui` da layout, diffs de buffer y eventos de ratón resueltos; `crossterm`
  da teclado/ratón/truecolor. Es exactamente el stack de spotatui.
- C queda descartado: pagarías en ergonomía (strings, JSON, HTTP, async) lo que
  ahorras en arranque, y el cuello de botella es el render, no el lenguaje.

Python sigue en el proyecto **solo** para herramientas externas del ecosistema
(yt-dlp), igual que ffmpeg.

Dependencias núcleo (versiones verificadas a 13-nov-2026): ratatui 0.30,
crossterm 0.29, tokio 1.54, rusqlite 0.42 (bundled), axum 0.8, reqwest 0.13
(rustls), image 0.25, rustfft 6.4, zbus 6.2, serde 1.0.

---

## 4. Identidad de canción y contrato `TrackRef`

Es el contrato más importante del proyecto. Si cambia, se toca todo.

```json
{
  "key": "isrc:USUM71703861",          // o "sha1:9f2c…" o "spotify:track:ID"
  "title": "Do I Wanna Know?",
  "artist": "Arctic Monkeys",
  "album": "AM",
  "duration_ms": 272000,
  "art_url": "https://…",              // puede ser file:// local
  "source_hint": { "provider": "ytmusic", "id": "abc123", "duration_ms": 272000 }
}
```

Reglas:

- `key` = ISRC si existe; si no `sha1` de `normalize(artist)+normalize(title)+
  round(duration_ms/1000)`; para lo importado de Spotify, `spotify:track:ID`
  para poder deduplicar contra el catálogo.
- `normalize()`: minúsculas, sin acentos, sin paréntesis de "(Official Video)",
  "(feat. …)" se separa a artistas adicionales, sin sufijos `- Remastered`.
- El servidor **solo** mueve `TrackRef` y estado. Nunca rutas locales.
- Cada cliente mantiene `track_key → source local` en su SQLite.
- En salas, un guest prueba primero el `source_hint` del host; si no, busca y
  **descarta candidatos fuera de ±2 s**.

---

## 5. Núcleo local

### 5.1 Librería

SQLite (`~/.local/share/spote/spote.db`) en WAL, con FTS5:

```sql
track(id, key, title, artist, album, duration_ms, art_path, added_at)
source(id, track_id, provider, provider_id, url, ext, bitrate,
       status, path, size, checksum)
lyrics(track_id, lrc, synced, source, offset_ms, fetched_at)
play_event(id, track_id, ts, ms_played, completed)
playlist(id, name, created_at)
playlist_item(playlist_id, track_id, position)
room_cache(...)   -- último estado de sala para arrancar sin red
setting(key, value)
```

`track` es canónico, `source` es materialización. Si el vídeo de YouTube muere,
se re-resuelve el source sin perder playlists ni historial.

### 5.2 Catálogo y descarga

- **Búsqueda**: `yt-dlp --flat-playlist --dump-single-json "ytsearch12:<q>"`
  (async, con debounce 250 ms). Filtra por duración y canal `- Topic`, puntúa
  con la heurística tipo spotDL (título normalizado + artista + duración).
- **Descarga**: `yt-dlp -f bestaudio -x --audio-format opus` a `media/<key>.opus`
  con `.part` y sidecars `<key>.jpg`, `<key>.lrc`, `<key>.json`.
- **Cola**: 2 workers, prioridad (lo que suena > siguiente > resto), cancelable
  (`kill` sin huérfanos). Prefetch de 2 pistas.
- **Reproducción mientras descarga**: mpv abre el `.part`; con Opus/WebM el
  decodificador tolera el fichero incompleto. Si aún no hay suficientes bytes,
  se espera. Nunca se reproduce un fichero de otra pista.
- **Importar playlist de Spotify**: leer páginas `/embed/` (sin auth, idea de
  Sunnify, reimplementada), crear `TrackRef` con `spotify:track:ID` y resolver
  cada uno contra ytmusic. Solo rellena huecos: no baja lo que ya está.
  Implementado: `spote import <url> [--download] [--dry-run]`
  (`crates/core/src/catalog.rs`). Acepta playlist, álbum, **artista** y pista.
  Límite conocido: el embed de playlist devuelve **50 pistas** como máximo; el de
  artista da sus 10 temas destacados y la imagen, pero **no la discografía**
  (ver §6.1).
- **Matching en el worker**: `Downloader::resolve_and_download(key, artista,
  título, duración, dir)` busca con yt-dlp, elige con `catalog::pick_best` y
  descarga, todo en el hilo de descargas. Lo usan la CLI y la TUI, así que una
  pista del catálogo no necesita URL previa.
- **yt-dlp**: tiene que ser reciente. La versión de apt (2025.04) falla al
  descargar con `The page needs to be reloaded`; comprobado bueno con 2026.10.26
  (`~/.local/bin/yt-dlp`, que va antes en el `PATH`).
- **Grabar lo que no existe en ninguna fuente**: `spote record` captura la salida
  del sistema (monitor de PipeWire) a 1×, con `parec` en float32 para no recortar
  (el material decodificado puede pasar de 0 dBFS), recorta el silencio inicial,
  lleva el pico a −1 dBFS y lo guarda como Opus 160 o FLAC. La pista entra en la
  librería con la misma clave `spotify:track:<id>`, así que una pista pendiente de
  `spote import` pasa a ser reproducible. Flags: `--flac`, `--from-now`, `--seconds N`.
- **Ingest**: `spote ingest <dir>` escanea audio local (incluido lo que produzca
  el extractor de Spotify), lee tags con `ffprobe`/`lofty`, calcula `key`,
  copia/renombra a `media/`, indexa. Es la puerta de entrada de cualquier audio
  que no venga de yt-dlp.

### 5.3 Motor de reproducción

`mpv --idle --no-video --input-ipc-server=<socket>` + JSON IPC:

- `loadfile`, `set_property pause|volume|speed`, `seek`, `get_property
  time-pos|duration|pause|idle-active|volume|media-title`.
- Suscripción a `time-pos`, `pause`, `eof`, `metadata`.
- `speed` con pitch correction (mpv lo hace por defecto) para el drift de salas.
- Al cambiar de pista: `stop` limpio, precarga de la siguiente `loadfile …
  append`, y `--gapless-audio=yes`.

### 5.4 Letras (portado de lyricbar)

**Implementado** en Rust: LRCLIB (sincronizado y plano), Genius (letra plana),
captions de YouTube y ASR local con faster-whisper. El reparto de letras
planas, la alineación fonética, las relecturas y la caché son de spote; no hay
dependencia de ningún checkout externo.

Base: **LRCLIB** (`/api/get`, `/api/search`, `/api/get/{id}`), gratis y sin auth.
Fallbacks: letra plana de Genius (reparto aproximado) y captions de YouTube,
ya integradas en el motor interno.

Lo que se reutiliza de un checkout local de lyricbar (Python, ~11k líneas) es el
**conocimiento**, no el código:

- `lyricbar/lrc.py` — parser LRC (incluye tags `<mm:ss.xx>` por palabra),
  inserción de líneas `♪` en huecos instrumentales, interpolación.
- `lyricbar/lyrics/service.py` — cadena LRCLIB → Genius → captions, matching
  por duración y caché.
- `lyricbar/sync.py` + `sync_runtime.py` — ajuste de offset con onsets/actividad
  vocal, caché de tiempos refinados.
- `lyricbar/audio.py` — FFT con ventana Hann, bandas log 40 Hz–16 kHz, attack/
  release suavizados, AGC. **Este archivo es la especificación exacta del
  `spectrum` de spote.**
- `lyricbar/ui/tui.py` — idea de línea actual resaltada + interpolación de
  progreso de frase.

La letra se cachea en SQLite y como `.lrc` junto al audio. Offset por track
(las versiones de YouTube nunca cuadran exacto).

### 5.5 Portada → paleta → tema

La portada manda. Pipeline:

1. Descargar/leer imagen (jpg/png/webp) y hacer thumbnail 64×64.
2. Bins de 4 bits por canal (4096), puntuar cada bin por
   `count * (0.15 + saturación) * preferencia de brillo medio` — así gana "el
   color del álbum" y no el borde negro ni el texto blanco. (Enfoque de
   spotatui `core/cover_theme.rs`, reimplementado.)
3. `Palette`: `primary`, `secondary` (hue a ≥60°, fallback a contraste de
   brillo, fallback al primary), más roles derivados: `bg`, `bg_deep`,
   `surface`, `surface_alt`, `border`, `text`, `muted`, `dim`, `accent`,
   `accent_alt`, `error`, `ok`.
4. **Legibilidad primero**: el texto nunca depende de la portada. Los acentos se
   clampean (`v ≥ 0.55` sobre fondo oscuro), el fondo se tiñe un 6-10 % del
   primary y se mantiene por debajo de luma 0.12. Contraste mínimo texto/fondo
   ≥ 7:1 (chequeo programático, no a ojo).
5. **Rampa**: 8-10 colores interpolados en OKLab entre `bg_deep` → `primary` →
   `secondary` → `text`. Se usa para el ascii art, los degradados de barras y la
   llama.
6. **Transición entre canciones**: lerp de toda la paleta en 800 ms con easing
   in-out-cubic, independiente del tick rate. Nunca un cambio brusco.

### 5.6 Cover art en la terminal

Modos (tecla `f` cicla):

- **`Half`**: bloques `▀` con fg = píxel superior, bg = píxel inferior
  (resolución vertical ×2, la mejor fidelidad). Con oscurecido por viñeta y
  gradiente vertical para que el texto respire.
- **`Dither`**: imagen reducida a la rejilla de caracteres, ordenada con matriz
  de Bayer 8×8 y cuantizada a la rampa de la paleta, usando `░▒▓█` + espacio.
  El look "TUI" más de verdad, pero sobre una foto grande genera ruido: se usa
  como modo artístico, no como default.
- **`Braille`**: umbralización 2×4 por celda con braille, tinta plana del
  acento. Más "plotter", menos foto.
- **`Gradient`**: degradado de la paleta + inicial del artista. Es el estado
  "sin portada" y también un modo artístico más.
- **Default: `Half`.** En una portada grande es el único que se lee como imagen
  (2 píxeles por celda) y sigue siendo "pixelado" a la vista. `A` cicla los
  cinco modos y `r` activa la rotación automática (cada 24 s por defecto) con
  disolución ordenada (Bayer) entre el arte viejo y el nuevo. Dither y braille
  se muestrean a 2×2 celdas cuando el panel es grande, para que el patrón se vea
  como pixel art y no como estática.
- **`Off`**: solo gradiente de fondo.

El arte se cachea en disco por `track_key` + tamaño + modo (`~/.cache/spote/art`),
así cambiar de canción no recalcula.

---

### 5.7 Discord Rich Presence

`crates/core/src/discord.rs` — IPC por socket Unix, sin dependencias nuevas.

- Busca el socket en `$XDG_RUNTIME_DIR`, `$TMPDIR` y `/tmp`, incluido Flatpak
  (`app/com.discordapp.Discord`) y Snap. Frames `u32 op` + `u32 len` + JSON.
- Handshake `op 0`, actividad `op 1` (`SET_ACTIVITY`). Si Discord contesta con
  error (`4000 Invalid Client ID`) se reporta tal cual.
- Actividad: `details` = título, `state` = `artista · álbum`, `timestamps.start`
  (Discord cuenta el tiempo solo), `timestamps.end` (barra de progreso),
  `assets.large_image` = **portada de Spotify por URL**, `small_text = spote` y
  botón "Abrir en Spotify" cuando la pista tiene id.
- Hilo propio: la UI nunca se bloquea. Reconexión perezosa (si Discord se
  reinicia, vuelve a conectar en la siguiente actualización) y límite de 2 s
  entre envíos (Discord permite ~5 cada 20 s).
- La marca de tiempo se recalcula al cambiar de pista **y al saltar** (si la
  posición real se desvía >2 s de la esperada).
- Se apaga al pausar: sin reproducción, sin actividad.

**Falta el Application ID.** Discord exige una app propia (gratis, 2 minutos):
<https://discord.com/developers/applications> → New Application → copiar
*Application ID* (no es un token) a:

```toml
[discord]
enabled = true
client_id = "1234567890123456789"
buttons = true
small_image = "spote"   # nombre del asset cuando se suba el logo
```

El nombre y los assets que se ven en el perfil salen de esa app. Cuando exista
el logo de spote se sube en *Rich Presence → Art Assets* con el nombre `spote` y
se activa `small_image`. Puesta en marcha paso a paso, problemas típicos y qué
límites tiene: **[`docs/discord-rich-presence.md`](docs/discord-rich-presence.md)**.
Resumen:

```bash
spote discord --id <application-id> --save   # guarda el ID y prueba la conexión
```

## 6. Spotify: qué hay y qué no (estado verificado)

Esto lo investigó otro agente en vivo contra el cliente oficial (cuenta free,
Linux, 12-nov-2026). Es información dura, no teoría:

**Verificado que sí:**

- La canción **entera** aterriza cifrada en `~/.cache/spotify/Data/` en segundos
  (58 ficheros de 3-5 MB, uno por pista, entropía 8.0, sin `OggS`).
- El cifrado es **AES-128-CTR** con el IV fijo de librespot
  (`72e067fbddcbcf77ebe8bc643f630d93`), presente en el binario oficial.
- El cliente **descifra en RAM**: se reconstruyeron páginas Ogg contiguas
  válidas (Vorbis 160 kbps, 44.1 kHz, CRC y granule correctos) desde memoria
  anónima. `ffprobe` las valida. O sea: leerle la RAM funciona.
- La clave se pide por HTTPS: `POST <region>-spclient.spotify.com/playplay/v1/key/<file_id>`
  con cuerpo protobuf (versión 5 + REQK + timestamp) y cabeceras `Authorization` y
  `client-token`, ambas presentes en memoria. Con el cuerpo correcto el proxy deja de
  responder 400 y pasa a 401 cuando el token caduca: **la cuenta free autentica**.
- **Pero la respuesta no es la clave AES**: devuelve `R1` (clave envuelta) + `R2`
  (parámetro de derivación). Desenvolverla es un paso nativo ofuscado y pinneado a la
  versión del cliente; los proyectos que lo hacían están tumbados por DMCA.

**Verificado que no:**

- La clave no está en RAM como array contiguo de 16 bytes: **~4·10⁹ candidatos** probados
  (cruda, como hex, pares `K‖IV`, IV fijo y contador a cero, prefijos de 0/4/8/16/24/32/
  64/128/167 bytes) con oráculo exacto y validado con un caso sintético → 0 aciertos.
  El descifrado no pasa por OpenSSL (gdb: `EVP_DecryptInit_ex` y `CTR128_encrypt` no se
  llaman durante la reproducción), está inlinado.
- La caché en disco no se puede descifrar: es el blob cifrado del CDN y la clave no
  persiste en ningún sitio.
- La cabecera de `0xA7` (167) bytes delante del Ogg es dato de terceros, **no confirmada**
  (se probó como offset en los escaneos, sin éxito).
- Recolectar páginas Ogg de la RAM funciona (Ogg Vorbis válido, `ffprobe` lo confirma)
  pero cubre un 15-20 % por pista y no es estable entre barridos.
- **Los saltos no aceleran la extracción.** Medido con `SetPosition`: después de un
  salto sólo aparecen ~10 páginas (~2 s de audio) en la ventana nueva, y el
  cliente descifra **justo a tiempo**. Escaneando en 0,2 s (sólo las regiones
  candidatas) tampoco se captura más: no hay buffer que adelantar. La RAM no
  puede ganar a 1×, así que no sirve de descargador rápido.
- **Conclusión:** para exclusivas, la vía es grabar la salida (`spote record`),
  que sí las saca al 100 % a 1×; o `spote harvest` si aceptas 97-99 % a 2-4×.

### 6.1 Artistas: hay retrato y temas, no discografía

El embed de artista (`/embed/artist/<id>`) sí da:

- `name`, `id`, imagen (`visualIdentity.image[0].url`)
- `trackList` con **10 temas destacados** (mismo formato que playlist/álbum)

Lo que **no** da, y se probó por varias vías:

- **Álbumes.** La página normal del artista (`open.spotify.com/artist/<id>`) se
  sirve vacía: no lleva `__NEXT_DATA__` ni IDs de álbum, tampoco con
  user-agent de Googlebot o Facebook. La API oficial exige app con Premium y el
  token anónimo del web player (`/get_access_token`) devuelve **403**.
- Alternativa que sí funcionaría y no se implementó: **Deezer**
  (`api.deezer.com/search/artist` → `artist/<id>/albums`) como fuente de
  discografía. Requiere mapear cada álbum de vuelta a Spotify, así que aporta
  títulos pero no IDs utilizables.

En la app, un artista se trata como colección de sólo temas: `spote import
<url-de-artista>` indexa sus 10 temas sin crear playlist.

**Consecuencia de diseño (importante):** spote **no depende** de esto. La app
funciona con yt-dlp y con ficheros locales. Cuando el extractor produzca Ogg
válidos (por RAM, por clave, o grabando), se meten con `spote ingest` y punto.
El formato de salida que espera `ingest`:

```
<dir>/<artist>/<album>/<nn> - <title>.ogg     (o .opus/.flac/.m4a/.mp3)
<dir>/<artist>/<album>/cover.jpg
<dir>/manifest.jsonl                           (opcional)
  {"file":"…","title":"…","artist":"…","album":"…","duration_ms":214000,
   "isrc":"…","spotify_id":"…","art":"…"}
```

**Ya implementado** (por el agente que trabajó en paralelo, 13-nov):

- `spote import <url-spotify> [--download] [--dry-run]` — lee playlists, álbumes
  y pistas públicas de las páginas `/embed/`, sin cuenta ni token, y opcionalmente
  resuelve y baja cada pista. Código: `crates/core/src/catalog.rs`.
- `spote record [--flac] [--from-now] [--seconds N]` — graba la salida del
  sistema (parec + ffmpeg) y corta por pista usando MPRIS. Es el único camino
  para contenido que no existe en otra fuente (Spotify Singles, podcasts).
  Coste: 1× tiempo real. Código: `crates/core/src/record.rs`.

Además hay un modo **`spote --follow-spotify`**: lee por MPRIS lo que suena en
el cliente oficial y pinta la TUI completa (portada, letras, visualizador) sin
tener el audio. Es la demo instantánea y el banco de pruebas del render. El audio
para el visualizador sale del monitor del sistema (`parec`/`pw-record`), igual
que en lyricbar. No requiere ningún crack.

---

## 7. La TUI (el corazón del proyecto)

Objetivo: que cuando alguien la vea diga "¿qué es eso?". Look de terminal de
verdad — retícula, bordes finos, tipografía mono, densidad — pero **todo
clickable, todo animado y todo reaccionando a la música**. Referencia de
producto: `spotatui`. Referencia de efectos: `cli-viz`.

### 7.1 Layout responsive (implementado)

```
wide (≥118 cols)
┌ ◈ spote   ◉ ahora  / buscar  ≡ librería  ⌂ sala ──────── ◆ sala  12:04 ┐
│ ┌ nav ─────┐ ┌ contenido ───────────────┐ ┌ portada ────┐ ┌ cola ────┐│
│ │ ◉ ahora  │ │ AHORA SUENA   ▁▂▅▃       │ │              │ │ 1 · …    ││
│ │ / buscar │ │ Título                   │ │   cover      │ │ 2 · …    ││
│ │ ≡ librer.│ │ artista · álbum          │ │   cuadrada   │ │          ││
│ │ ⌂ sala   │ │ ──────────────────────── │ │   2:1       │ │          ││
│ │          │ │     letra anterior       │ │              │ │          ││
│ │ playlist │ │   ▸ letra actual ◂       │ │              │ │ sesión   ││
│ │  · …     │ │     letra siguiente      │ │              │ │ ──────── ││
│ └──────────┘ └──────────────────────────┘ └──────────────┘ └──────────┘│
│ ◀◀  ⏸  ▶▶   ♪ ▮▮▮▮▮▮▯▯▯▯  1:23 / 4:32          arte half  aurora      │
└────────────────────────────────────────────────────────────────────────┘
```

- **Portada siempre cuadrada de verdad**: la celda es ~1:2, así que el rect es
  `w = 2·h`. Se pide al renderer en ese tamaño y no se estira nunca.
- `< 118` cols o poca altura: la portada pasa a **miniatura dentro del panel**
  (arriba a la derecha) y el texto ocupa todo el alto, para que la letra no
  desaparezca. El nav desaparece por debajo de 92 cols.
- `≥ 118`: nav + contenido + portada + cola + sesión.
- El layout se recalcula **por frame**; no hay estado de "modo móvil".
- `F` pone la portada a pantalla completa; `V` el visualizador.

### 7.2 Capas de la pantalla (de atrás adelante)

0. **Fondo**: gradiente vertical `bg_deep → bg` + viñeta. Barato, se cachea por
   tamaño + paleta.
1. **Arte**: cover en ascii/dither, con oscurecido y máscara (radial o
   gradiente) para que el contenido encima se lea. Modo fullscreen con `f`.
2. **Visualizador**: N capas compuestas (ver §7.4), recortadas a la zona
   asignada (detrás del arte, franja inferior, o fullscreen con `v`).
3. **UI**: paneles con bordes, textos, letras, cola.
4. **Overlays**: toasts, ayuda, buscador, selector de salas, menú de efectos.
5. **Cromo**: barra superior y transporte, con animaciones.

### 7.3 Interacción (todo clickable)

Cada frame registra `Rect → Action` en un `HitMap`. El ratón consulta el mapa:

- Click en tabs/paneles → foco. Click en fila de cola → play.
- Click/arrastre en la barra de progreso → seek (con preview de tiempo bajo el
  cursor). Rueda sobre la cola/letra → scroll. Rueda sobre el volumen → volumen.
- Click en el arte → fullscreen. Click en el visualizador → siguiente efecto.
- Hover → el borde se ilumina y el fondo sube un escalón (feedback inmediato).
- Doble click en una fila → añadir a la cola.
- Botones del transporte con estados hover/active y "pressed" (200 ms).

Teclado completo y documentado en `?`: `space`, `←/→` ±5 s, `⇧←/→` ±30 s,
`↑/↓` navegar, `enter` play, `a` añadir a cola, `/` buscar, `tab` foco,
`1-9` efectos, `v` ciclar, `V` fullscreen viz, `f` fullscreen arte, `l` foco
letra, `s` salas, `m` mute, `+/-` volumen, `q` salir.

### 7.4 Sistema de visualización (port de cli-viz, combinable)

`cli-viz` demuestra 15 efectos en curses. Aquí no se copia código (Python,
curses, sin composición); se porta el **mecanismo** a un trait común:

```rust
trait Effect {
    fn name(&self) -> &'static str;
    fn update(&mut self, dt: f32, a: &AudioFrame, ctx: &VizCtx);
    fn render(&mut self, canvas: &mut Layer);   // RGB + char + blend
    fn params(&mut self) -> &mut Params;        // sensibilidad, densidad, …
}
```

`AudioFrame` (no es el spectrum crudo, es lo que necesitan los efectos):

```rust
struct AudioFrame {
    bands: [f32; 64],      // log-spaced, 40 Hz..16 kHz, suavizado attack/release
    bass: f32, mid: f32, treble: f32,
    energy: f32,           // RMS normalizado
    beat: bool,            // onset por historial + umbral
    beat_pulse: f32,       // 0..1 con decay
    waveform: [f32; 256],  // osciloscopio para wave/tunnel
    hue: f32,              // fase lenta global
    palette: Palette,
    time: f32,
}
```

**Composición por capas** (esto es lo que pedía "uno u otro o combinados"):

```rust
struct Viz {
    layers: Vec<Layer>,          // hasta 6
}
struct Layer {
    effect: Box<dyn Effect>,
    blend: Blend,                // Normal | Add | Screen | Multiply | Overlay
    opacity: f32,                // 0..1
    mask: Mask,                  // Full | Radial | BottomGradient | Band(lo,hi)
    enabled: bool,
}
```

Presets con nombre (tecla `S`): `aurora` (wave + particles + starfield, screen),
`inferno` (flame + bars, add), `void` (starfield + matrix, multiply),
`bloom` (circle + pulsar, screen), `glitch` (fractal + bars, overlay),
`clean` (solo bars).

Efectos portados/inspirados de cli-viz (todos reimplementados, con la paleta
del álbum en vez de HSV aleatorio):

| # | efecto | mecanismo que se porta |
|---|--------|------------------------|
| 1 | `bars` | spectrum no lineal (`i^1.3`), bass boost, gradiente por altura |
| 2 | `wave` | suma de senos con amplitud/fase por banda + osciloscopio |
| 3 | `circle` | anillos radiales con radio = energía, chars por energía |
| 4 | `particles` | spawn por beat, gravedad, vida, char por velocidad |
| 5 | `flame` | buffer de calor + difusión + ruido, altura según bass |
| 6 | `matrix` | columnas con cola de chars, velocidad por banda, glifos katakana |
| 7 | `starfield` | proyección 3D al centro, warp por energía, estelas |
| 8 | `fractal` | Mandelbrot/Julia animado, paleta por iteración, zoom por beat |
| 9 | `neural` | nodos en rejilla + aristas por proximidad, pulso por banda |
| 10 | `pulsar` | rayos radiales + anillos expansivos por beat |
| 11 | `tunnel` | túnel de anillos en perspectiva, radio por bass |
| 12 | `spectro3d` | barras con perspectiva (falso 3D), profundidad por banda |

Implementados en `crates/tui/src/viz/effects.rs`: 12 efectos (`bars`, `wave`,
`circle`, `particles`, `flame`, `matrix`, `starfield`, `fractal`, `neural`,
`pulsar`, `tunnel`, `spectro3d`) con el compositor de capas, blends
(`normal/add/screen/multiply/overlay`), máscaras (`full/radial/bottom/band`) y
7 presets. `docs/effects.md` tiene la especificación del port, incluidos los que
aún no están (neural-lite, stick figure, tuner descartado).

Reglas de rendimiento:

- Canvas de efectos en resolución de celda (no sub-celda) y solo en su `Rect`.
- 60 fps objetivo; si un frame tarda >8 ms, baja a 30 fps solo y avisa en el
  footer (`fps` visible, como los emuladores).
- Estado de efectos en `Vec<Box<dyn Effect>>` reutilizado, cero allocs por frame
  (buffers `Vec<Cell>` con capacidad reservada).
- Solo se repinta cuando hay cambio visual o audio activo; en pausa, 10 fps.

### 7.5 Animaciones y "feeling"

- **Springs** para foco/hover/scroll (masa-muelle-amortiguación, dt real).
- **Theme fade** 800 ms al cambiar de pista (§5.5).
- **Letra karaoke**: la línea actual crece en opacidad y brillo, la siguiente
  entra desde abajo con spring; el progreso por palabra se pinta con un degradado
  (`primary` → `text`) sobre los caracteres ya cantados.
- **Beat**: en `beat`, el borde del panel activo sube 8 % de brillo y el playhead
  hace un pulso de 150 ms. Nada de flashes epilépticos.
- **Toasts**: entran deslizando desde abajo-derecha, 3 s, se van con fade.
- **Arranque**: 400 ms de fade-in del arte + barras del viz desde 0. Sin logos
  ASCII gigantes ni pantallas de carga.
- **Cursor**: bloque fino, parpadeo lento solo en inputs.
- Todo medido con tiempo real (`Instant`), nunca con ticks, para que el feel no
  dependa de la carga.

### 7.6 Paleta visual por defecto (sin portada)

Fondo `#0b0b0f`, superficie `#12121a`, texto `#e8e6f0`, muted `#8a879c`, acento
`#c8b6ff` (lila), alt `#7ee0c3` (menta). Deliberadamente neutro para que la
portada lo tiña todo. Si la portada es gris, el acento cae al lila por defecto:
nunca una UI gris.

---

## 8. Backend social (`spoted`)

Implementado y probado (9 tests: REST, servicio, WebSocket real). Axum + SQLite
WAL + WS. Sin audio, sin yt-dlp, sin disco. La TUI se conecta por WebSocket,
hace login por invitación, lista salas, se une, publica presencia y sincroniza
play/pause/seek con el estado de la sala.

### 8.1 REST

```
POST /api/v1/auth/login      {name, invite}        → {token, user}
GET  /api/v1/me                                    → user
GET  /api/v1/users           (amigos + presencia)
POST /api/v1/friends/{id}
GET  /api/v1/rooms                                 → salas visibles
POST /api/v1/rooms           {name}                → room
POST /api/v1/rooms/{id}/join | /leave
GET  /api/v1/rooms/{id}                            → snapshot
POST /api/v1/rooms/{id}/control {action, position_ms?}
POST /api/v1/rooms/{id}/queue   {track: TrackRef}  → queue_item
DEL  /api/v1/rooms/{id}/queue/{item}
GET  /api/v1/time                                  → {t}  (sync de reloj)
```

### 8.2 WebSocket `/api/v1/ws?token=…`

Mensajes (JSON, `type` como tag):

- cliente→servidor: `hello{device, now_playing}`, `presence{now_playing}`,
  `ping{t0}`, `join{room}`, `leave{room}`, `control{…}`, `queue_add{…}`,
  `ready{track_key, buffered_ms}`, `chat{text}`.
- servidor→cliente: `welcome{server_time}`, `pong{t0,t1}`, `presence{user,…}`,
  `room_state{track, playing, position_ms, anchor_ms}`, `queue{items}`,
  `peer_ready{…}`, `chat{…}`, `start_at{track, at_ms}`.

### 8.3 Reloj y sincronización

Estado de sala: `{track, playing, position_ms, anchor_ms}`.
Cliente: `server_now = client_now + offset`;
`target = position_ms + (playing ? server_now - anchor_ms : 0)`.

`offset` por NTP casero: 5 pings, quedarse con el de menor RTT.

Deriva `d = target - posición_real`:
`|d| > 400 ms` → seek · `40–400 ms` → `speed = 1 + d/8000` (clamp 0.97–1.03)
durante ~8 s · `< 40 ms` → nada. mpv corrige el pitch.

Cambio de pista: el servidor anuncia `start_at` absoluto ~8 s antes; los
clientes precargan y cambian todos en ese epoch. `ready` con timeout de 4 s.

### 8.4 DB del servidor

```sql
user(id, name, avatar, invite_code, created_at)
friend(user_id, target_id, accepted)
room(id, name, host_id, created_at, is_public)
room_member(room_id, user_id, role)
room_state(room_id, track_json, position_ms, playing, anchor_ms, updated_by)
queue_item(id, room_id, track_json, added_by, position, votes)
chat(id, room_id, user_id, text, ts)
play_event(id, user_id, track_json, ts, ms_played)   -- feed
```

Presencia en memoria (`user_id → last_seen, now_playing`), volcada a DB cada 30 s.

### 8.5 Auth

Invitación (código por usuario) + token Bearer opaco en tabla `session`.
Sin OAuth, sin emails, sin password. 4 amigos.

---

## 9. Rendimiento y calidad

- Presupuesto de frame: **8 ms** a 60 fps. Medido con un overlay de debug (`F3`:
  fps, ms del último frame, celdas, capas activas).
- Sin allocs en el hot path del render (buffers reutilizados).
- El spectrum corre en su propio thread (decode + FFT), la UI solo lee un
  `Arc<AudioFrame>` con `RwLock` o un channel con el último frame.
- Descargas y HTTP en tokio; la UI es single-thread con `crossterm::event::poll`
  de 8 ms.
- Nunca bloquear el render con red o disco: todo lo lento va a un worker y
  reporta por channel.

---

## 10. Config

`~/.config/spote/config.toml`:

```toml
[server]
url = "https://spote.example.com"
token = "…"

[player]
volume = 0.8
prefetch = 2

[ui]
art_mode = "dither"       # half | dither | braille | off
viz_default = "aurora"
theme_fade_ms = 800
fps = 60
show_fps = false

[download]
format = "opus"
quality = 0               # bestaudio
dir = "~/.local/share/spote/media"
```

Todo con defaults sensatos: `spote` sin config funciona (modo solo).

---

## 11. Roadmap

- **Fase 0 (esta sesión)**: workspace Rust, `idea.md`, core (config, library,
  player, lyrics, artwork, spectrum), TUI completa con arte/paleta/efectos/
  ratón/animaciones, servidor social funcional, `--follow-spotify`.
- **Fase 1**: búsqueda real (ytmusic), descarga con cola y cancelación,
  importador de playlists, `ingest` probado con audio real, salas end-to-end
  con dos clientes.
- **Fase 2**: web (misma API), overlay de escritorio (reutilizar el de lyricbar),
  scrobble a Last.fm, ecualizador, radio infinita.
- **Fase 3**: Android (Kotlin+Media3) con `track_ref` y protocolo congelados;
  cache LRU, offline, descarga por álbum.
- **Fase 4**: votos/DJ rotatorio, letras con captions, temas compartidos.

---

## 12. Preguntas abiertas (para Fabio)

1. **Nombre**: ¿`spote` se queda? Afecta a binarios, DB, config y URLs.
2. **Modo Spotify**: ¿la TUI debe poder ser "acompañante" del cliente oficial
   (follow + letras + visuales) como modo de primera clase, o solo un extra?
3. **Fuente por defecto**: ¿YT Music Topic-first (mejor para salas) o mezclar
   con SoundCloud en la misma búsqueda?
4. **VPS**: ¿dominio ya decidido y TLS? ¿Lo despliego con Docker Compose o
   binario + systemd?
5. **Diseño**: ¿prefieres el arte en `dither` (más TUI, más "pixelado") o en
   `half-blocks` (más fiel)? ¿O que el default sea dither y `f` cicle?
6. **Sonido en salas**: ¿toleras el micro-ajuste de velocidad (0.97–1.03) para
   no hacer seeks, o prefieres seek seco cuando hay deriva?
7. **Letras**: ¿karaoke por palabra solo cuando haya tags, o forzamos estimación
   por sílabas siempre?
8. **Scope v1**: ¿meto ya la web o me quedo solo en TUI + servidor esta ronda?

Las respuestas se van anotando aquí. Mientras no haya respuesta, se aplican los
defaults de este documento.

---

## 13. Referencias

| Qué | Para qué |
|---|---|
| `https://github.com/sam1am/cli-viz` | Los 15 efectos de visualización. Mecanismos, no código. |
| `https://github.com/LargeModGames/spotatui` | Producto y arquitectura: core/tui/gui, listening party, friends, `core/cover_theme.rs`. |
| `https://github.com/spotDL/spotify-downloader` | Matching título/artista/duración, providers con fallback, manifest. MIT. |
| `https://github.com/sunnypatell/sunnify-spotify-downloader` | Import de playlists por embed pages sin auth. Licencia custom: reimplementar la idea, no copiar. |
| `https://github.com/WilliamSchack/Spotify-Downloader` | Checklist operativa (rate limits, PO tokens, cookies). GPL: no copiar. |
| `lrclib.net` | Letras sincronizadas gratis, sin auth. |
| `mpv` IPC docs | Motor de reproducción y `speed` con pitch correction. |
| `librespot` (`audio/src/decrypt.rs`) | AES-128-CTR, IV fijo. Contexto del cifrado de Spotify. |

---

## 14. Convenciones para futuros agentes

- **Lee esto entero antes de tocar código.** Si cambias una decisión de §2 o el
  contrato de §4, actualiza el documento en el mismo commit.
- **El contrato `TrackRef` y los mensajes WS son API pública** entre tres
  clientes. Cambiarlos rompe todo. Versiona (`/api/v1`).
- **La UI no bloquea.** Ninguna llamada de red/disco/yt-dlp en el hilo de render.
- **No metas dependencias nuevas sin justificar** en este documento.
- **La portada manda**: si añades un color hardcodeado en la UI, es un bug.
- **Verificación obligatoria**: `cargo build --workspace`, `cargo test`, y
  arrancar la TUI en tmux para comprobar que no peta a 80×24 y a 200×60.
  Captura de pantalla: `scripts/shot.sh` (tmux + ANSI→HTML + screenshot).
- **Presupuesto**: si tu cambio hace que un frame tarde >8 ms en 200×60, no
  entra.
- **Privacidad**: nada de nombres/datos personales en artefactos del repo.
- **Idioma**: código y commits en inglés; comentarios y docs en español.
- **No commitees** sin que lo pida Fabio.

---

## 15. Estado actual del repo (v0.1, 2026-11-13)

### Qué funciona hoy, verificado

- **Workspace Rust** (`cargo test --workspace`: 97 tests en verde; `cargo check
  --workspace` sin warnings). Binarios: `spote` y `spoted`.
- **Librería local** SQLite + FTS5: ingesta desde carpeta con tags de `ffprobe`
  (incluye Ogg/Opus, que guarda los tags en el stream), playlists, favoritas,
  historial, caché de letras y de arte.
- **Reproducción** con mpv por IPC: play/pause/seek/volumen, eventos de tiempo,
  `--ao=null` opcional para CI (`SPOTE_AO=null`), posición fiable.
- **Letras** LRCLIB reales: `/api/get` → `/api/search`, ranking por título,
  artista y duración, fallback a letra plana repartida, caché SQLite + `.lrc`
  local. Verificado con *Borderline* (50 líneas sincronizadas).
- **Portada → paleta → tema**: extracción por bins con puntuación de saturación,
  superficies con techo de luminancia, contraste texto/fondo ≥ 7:1, rampa de 12
  pasos monótona, fade de 800 ms entre canciones.
- **Arte** en cuatro modos (`half` por defecto, `dither`, `braille`, `off`),
  cacheado en disco, recoloreado por la paleta viva.
- **TUI**: 4 vistas (ahora, buscar, librería, salas), portada cuadrada grande,
  letra con karaoke y scroll con muelle, transporte, cola, sesión, overlays de
  ayuda y de capas, toasts, hover, clic en todo, arrastre en la barra de
  progreso, rueda, doble clic, responsive de 30×10 a 200×60, 60 fps.
- **Visualizador**: 12 efectos, compositor de capas con blend y máscaras, 7
  presets, FFT real (64 bandas log, AGC, beat) desde ffmpeg (fichero) o `parec`
  (audio del sistema).
- **Modo `--follow-spotify`**: lee MPRIS, descarga la portada de Spotify, pinta
  letra y estado, y usa el monitor del sistema para el visualizador. Probado
  en vivo con el cliente oficial.
- **Import de Spotify** (`spote import`): catálogo por embed pages sin cuenta,
  con `--download` para materializar la colección con yt-dlp. Probado con una
  playlist de 50 pistas.
- **Grabación** (`spote record`): captura del monitor del sistema con corte por
  pista vía MPRIS, en Opus o FLAC.
- **Backend `spoted`**: login por invitación, amigos, presencia, salas, cola,
  chat, control, `ready`/`start_at`, reloj. La TUI se une a una sala, publica
  presencia y sincroniza la reproducción (verificado end-to-end contra un
  servidor real: unirse → recibir `RoomState` → cargar el fichero local → seek).
- **Herramientas**: `scripts/frame.py` (captura la TUI en un pty, interpreta
  ANSI y genera HTML/PNG sin tmux), `scripts/ansi2html.py`, `scripts/shot.sh`.

### Cambios de la ronda 2 (13-nov, madrugada)

- Nombre configurable (`[app] name`).
- Modo de arte `gradient` + rotación automática con disolución + muestreo 2×2
  para dither/braille en paneles grandes.
- `spote` lleva su propio proveedor de letras (LRCLIB, Genius, captions de
  YouTube y ASR en Rust).
- Buscar escribe directamente; `/` solo sirve para entrar en la vista.
- Corazón visible en favoritas (lista, tarjetas y panel de ahora).
- Cola: quitar (`x`/`×`/clic), reordenar (arrastrar, `shift ↑↓`, `, .`).
- Lista y cola en **tarjetas** de 3 filas con miniatura (`z`).
- Analizador con bombo/caja/charles/voz y BPM; la UI y los efectos laten con
  cada instrumento.
- Visualizador como **fondo** de toda la TUI (`b`), paneles translúcidos.
- Clic en la portada o en la letra para que ocupen el centro (`c`, `C`).
- Arregladas las mayúsculas: `A` ya no añade a la cola, y el resto de hotkeys
  respetan shift.
- `run.sh` y repo git inicializado.

### Ronda 3 (13-nov, mañana)

- Gradiente fuera de la rotación: solo se usa cuando no hay portada.
- Arte robusto: se limpia al cambiar de pista, la petición grande tiene
  prioridad sobre las miniaturas, y en modo Spotify se reutiliza la clave/arte
  local por artista+título+duración.
- Foco de portada centrado de verdad; letra a todo el centro.
- Cabecera compacta (badges arriba, título + artista en una línea).
- Transporte centrado en el panel inferior; leyenda al pie.
- Zen (`i`): oculta portada/letra y deja el visualizador a pantalla completa.
- Pestaña de ajustes: rotación, segundos, modo de arte, tarjetas, fondo,
  centrado vertical, zen y Spotify.
- Búsqueda con metadatos de Spotify (client credentials o token anónimo) y
  Deezer como respaldo; descarga con yt-dlp a partir de esos datos.
- Seguir Spotify separado del modo local: al activar se pausa mpv; al
  desactivar se restaura la pista local. Seek/Next/Prev/Play van por MPRIS.
- Arreglada la congelación de letras: MPRIS manda la posición en cada sondeo y
  el reloj interpola sin cortar a los 3 s; solo se resincroniza con deriva real.
- Cola: el `×` gana al clic de la fila (un clic quita).
- Sala y biblioteca con el mismo ancho; filtros de favoritas/descargadas/
  recientes y playlists funcionando.
- Visualizador `clean` con degradado de color; pulsos de instrumentos en toda
  la UI (paneles, tabs, transporte, letra, cola).

### Ronda 4 (13-nov, tarde)

- `f` actúa sobre la selección de la pestaña activa; en ajustes/sala ya no
  marca la pista que suena. `a` también respeta búsqueda/biblioteca.
- `vcenter` centra el bloque de paneles (el cuadrado), no el texto; sin pista
  el panel se centra en la pantalla.
- Modo de arte `plain`: media celda con los píxeles originales, sin paleta,
  viñeta ni pulso. Entra en la rotación y en `A`.
- Ajuste de efecto de miniaturas: `palette`, `plain` o `off`.
- Transporte configurable: barra en el visualizador (centrada en la franja),
  ancho 55-100 % y tamaño de controles 1-4.
- Listas completas: crear con nombre/descripción/portada (URL o automática),
  borrar, añadir pistas, filtros con `[`/`]` y portada en la cabecera.
- Importación de playlists públicas de Spotify/Deezer con metadatos y descarga
  bajo demanda.
- Progreso real de descarga por pasos (`StageProgress` de yt-dlp) visible en
  sesión, transporte y filas.
- Refactor: `tui/ui` en 8 módulos y `tui/app` en 6; primitivas y `Layout` en
  `ui/mod.rs`; `docs/arquitectura.md` con las recetas de extensión.
- `buf.reset()` + recorte por caracteres; la búsqueda ya no revienta con
  acentos ni con el separador `·`.

### Nota sobre trabajo en paralelo

El repo lo están tocando dos agentes a la vez. En la sesión del 13-nov, además
de lo descrito aquí, se añadieron `crates/core/src/catalog.rs` (import de
Spotify por embed), `crates/core/src/record.rs` (grabación del monitor) y los
comandos `spote import` / `spote record` en `main.rs`. Si trabajas en el repo,
comprueba `ls -lt crates/core/src` y `git` (no hay) antes de reescribir ficheros
grandes: puede haber cambios no commiteados de otra sesión.

### Qué falta / siguiente

1. **Resolvedor de búsqueda real**: `yt-dlp ytsearch` ya está integrado, pero el
   matching fino (canal `- Topic`, duración ±2 s, descarte de covers) es básico.
   Portar la heurística de `spotDL` y escribir fixtures de canciones conocidas.
2. **Reproducción mientras descarga**: hoy se espera al `Done` del downloader.
   Falta abrir el `.part` en mpv en cuanto haya bytes, con prefetch de 2 pistas.
3. **Importador de playlists de Spotify** por embed pages (§5.2). No está.
4. **Salas**: falta UI de chat, votos, DJ rotatorio y `ready` handshake fino
   (la TUI no manda `Ready` todavía). El servidor ya los soporta.
5. **`ingest` del extractor de Spotify**: el pipeline está listo para cualquier
   audio; falta probarlo con los Ogg reales cuando el otro agente los produzca.
6. **Web / Android**: no empezados. El protocolo está congelado en §4 y §8.
7. **Tests de UI**: hay tests de layout, efectos y mezcla de paleta. Falta un
   test de integración que arranque la TUI en un pty y compruebe teclas.

### Cómo verificar sin oído

```bash
# 1. Tests
cargo test --workspace

# 2. Demo reproducible (track sintético + portada real)
mkdir -p /tmp/spote-demo/"Tame Impala"/"The Slow Rush"
# cover.jpg + 01 - Borderline.opus con tags (ver más abajo)
SPOTE_HOME=/tmp/spote-home ./target/debug/spote ingest /tmp/spote-demo

# 3. Captura de la TUI (sin tmux, sin X)
python3 scripts/frame.py --cols 200 --rows 56 --out /tmp/frame.html \
  --text-out /tmp/frame.txt \
  --actions "wait 4; key space; wait 3; key Right 8; wait 5" \
  -- env SPOTE_HOME=/tmp/spote-home SPOTE_AO=null ./target/debug/spote

# 4. HTML -> PNG
#    navegar a file:///tmp/frame.html y capturar a 2000x1140 (o chromium headless)
```

Generar el audio de demo (pulso para que el visualizador tenga señal):

```bash
ffmpeg -f lavfi -i "aevalsrc='0.3*sin(2*PI*110*t)*(0.55+0.45*sin(2*PI*2*t))+0.16*sin(2*PI*220*t)*(0.6+0.4*sin(2*PI*3*t))':s=44100:d=237" \
  -c:a libopus -b:a 96k -metadata title="Borderline" -metadata artist="Tame Impala" \
  -metadata album="The Slow Rush" "01 - Borderline.opus"
```

### Limitaciones conocidas

- El arte de `half` recibe una compensación de brillo (+28 %) porque el
  renderer aplica viñeta y gradiente pensando en fondos, no en imagen principal.
  Si se toca `render.rs`, ajustar también `draw_art`.
- En modo acompañante la posición se interpola entre sondeos de MPRIS (400 ms);
  no hay suspensión de la interpolación al pausar más allá de 3 s.
- Las salas aplican el estado del servidor con seeks secos (sin corrección de
  velocidad con pitch) — pendiente el lazo fino de ±3 %.
- La UI no manda `Ready` al unirse; el `start_at` del servidor no se usa aún.
- En una sala, poner una canción local la envía a la cola compartida (no la
  reproduce en solitario). Cambiar la pista de la sala es `n`/`Next` del host.
- El servidor limpia el `track` de la sala si se hace `next` con la cola vacía.
  Es comportamiento suyo, no de la TUI.
- `spote ingest` copia los ficheros al directorio de medios; no mueve.

### Preguntas de §12: respuestas por defecto aplicadas

1. Nombre: **spote** se queda (binarios `spote`/`spoted`).
2. Modo Spotify: **sí**, es de primera clase (`--follow-spotify` / tecla `t`).
3. Fuente por defecto: YT Music Topic-first; SoundCloud queda como fallback.
4. VPS: sin decidir; `spoted` acepta `SPOTE_BIND`, `SPOTE_DB`, `SPOTE_INVITE`.
5. Diseño: default `half`; `dither` y `braille` disponibles con `A`.
6. Salas: por ahora seek seco; el micro-ajuste de velocidad es el siguiente paso.
7. Letras: karaoke por palabra cuando hay tags `<mm:ss.xx>`; si no, progreso de
   línea.
8. Scope: esta ronda ha sido TUI + servidor; web y Android quedan para después.

> **Roadmap**: la fase 0 de §11 está completada. Las fases 1-4 siguen vigentes;
> el orden recomendado ahora es: resolvedor/matching → reproducción con fichero
> creciente → importador de playlists → salas completas → web → Android.
