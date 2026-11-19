# spote

**Tu música, en la terminal.** La portada tiñe la interfaz, los bordes laten con
el bombo y las letras van a tiempo. TUI en Rust, `mpv` para sonar, `yt-dlp`
para bajar y SQLite para la biblioteca.

![spote](docs/screenshots/now.png)

## Instalar

```bash
git clone https://github.com/fabiomode2/spote && cd spote
./scripts/install.sh
spote

# opcional: captions de YouTube
./scripts/install.sh --lyricbar ~/Code/lyricbar
```

Necesitas Rust 1.82+, `mpv`, `ffmpeg` y `yt-dlp` en el `PATH`. El instalador
deja los binarios en `~/.local/bin`, crea el venv con `faster-whisper` y copia
los scripts. Las letras van dentro: LRCLIB, Genius y ASR. No hace falta
lyricbar.

Actualizar: `./scripts/update.sh` o desde el aviso de versión de la propia TUI.

## Lo que mola

- **Arte vivo** — portada en half-block, dither, braille o imagen real en
  Kitty; la paleta del tema sale de la carátula y los bordes, el playhead y las
  letras laten con bombo, caja y voz.
- **Letras de verdad** — LRCLIB, Genius y ASR local. Lectura completa,
  relectura de la cola con 15 s de contexto y alineación fonética en Rust. Un
  LRC sincronizado bueno no se toca; el estimado se afina solo.
- **Karaoke** — palabra a palabra, frases largas en dos filas y la última
  palabra se llena antes de cambiar de línea.
- **Búsqueda sin cuenta** — Spotify y Deezer como catálogo; el audio sale de
  `yt-dlp`. Importa playlists y álbumes con `I`.
- **Cola y listas** — encolar, reordenar, descargar, favoritas y playlists
  locales. `z` alterna tarjetas con portada y lista compacta.
- **Salas** — `spoted` añade presencia, chat y cola compartida.
- **Todo en caliente** — `E` edita el layout (paneles, anchos, dock, tamaños) y
  se guarda solo en `config.toml`.

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

## Cómo suena

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

[ui.fx]
ambient = "cover"      # off | void | aurora | neural | glitch | lyrics | cover

[lyrics]
asr = true
asr_model = "small"    # tiny | base | small | medium | large-v3
```

`LYRICBAR_ROOT` solo hace falta si quieres captions de YouTube. El resto de
letras ya van dentro.

## Más

```bash
spote ingest ~/Music                            # indexa audio local
spote import <url-spotify> --download           # importa y baja
spote doctor                                    # comprueba dependencias
spote record [--flac]                           # graba lo que suena
spoted                                          # backend social en :7070
```

- `idea.md` — producto, arquitectura y decisiones.
- `docs/arquitectura.md` — módulos, layout y mapa de audio.
- `docs/effects.md` — especificación de los efectos.
- `docs/rework.md` — estado y limitaciones.
- `docs/screenshots/` — más capturas.
