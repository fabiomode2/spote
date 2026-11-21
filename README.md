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

Hacen falta Rust 1.82+, `mpv`, `ffmpeg` y `yt-dlp` en el `PATH`. El instalador
deja los binarios en `~/.local/bin`, crea el venv con `faster-whisper` y copia
los scripts. Las letras no necesitan nada más: LRCLIB, Genius, captions de
YouTube y ASR van dentro.

Para actualizar: `./scripts/update.sh`, o desde el aviso de versión que sale en
la propia TUI.

## Qué hace

- **Arte**: portada en half-block, dither, braille o imagen real en Kitty. La
  paleta del tema sale de la carátula.
- **Letras**: LRCLIB, Genius y ASR local. Lee la canción completa, relee la cola
  con 15 s de contexto y alinea la letra en Rust. Un LRC sincronizado bueno no
  se toca; el estimado se afina solo.
- **Karaoke**: palabra a palabra, frases largas en dos filas y la última palabra
  se llena antes de cambiar de línea.
- **Búsqueda**: Spotify y Deezer como catálogo, sin cuenta. El audio sale de
  `yt-dlp`. `I` importa playlists y álbumes.
- **Cola y listas**: encolar, reordenar, descargar, favoritas y playlists
  locales. `z` alterna tarjetas con portada y lista compacta.
- **Salas**: `spoted` añade presencia, chat y cola compartida.
- **Layout**: `E` edita paneles, anchos, dock y tamaños. Se guarda solo en
  `config.toml`.

## Atajos

| | | | |
|---|---|---|---|
| `space` | play/pausa | `a` | encolar |
| `n` / `p` | siguiente / anterior | `f` | favorita |
| `←` `→` | seek ±5 s | `g` | artista |
| `/` | buscar | `T` | recortar intro/outro |
| `u` | bucle de la pista | `F` | portada a pantalla completa |
| `c` / `C` | portada / letra | `V` | visualizador a pantalla completa |
| `E` | editor de diseño | `?` | ayuda |

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
viz = "clean"          # clean | aurora | bloom | inferno
fps = 60               # frames de la UI (20-60)
fx_fps = 60            # ritmo de los efectos; a pantalla completa va a la mitad

[ui.fx]
ambient = "cover"      # off | void | aurora | neural | glitch | lyrics | cover

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
