# spote

Reproductor de música privado con una TUI que reacciona a la portada del álbum.
Cliente local (descarga con `yt-dlp`, reproduce con `mpv`) + backend social
(`spoted`) para presencia, salas y cola compartida.

```
crates/
  protocol/   contrato compartido (TrackRef, salas, mensajes WS)
  core/       config · librería SQLite · descarga · mpv · letras · portada · audio
  server/     spoted: axum + SQLite + WebSocket
  tui/        spote: ratatui + crossterm
```

![spote](docs/screenshots/now.png)

## Requisitos

`mpv`, `ffmpeg` y `yt-dlp` en el `PATH`. Para el visualizador con audio del
sistema, `parec` (PipeWire/PulseAudio). Rust 1.82+.

## Compilar y ejecutar

```bash
cargo build --release
./target/release/spote                 # TUI (modo local)
./target/release/spote --follow-spotify # acompaña al cliente oficial por MPRIS
./target/release/spote ingest ~/Music   # indexa audio local
./target/release/spote import <url-spotify> --dry-run  # catálogo sin cuenta
./target/release/spote import <url-spotify> --download # importa y baja
./target/release/spote record [--flac]  # graba la salida del sistema
./target/release/spote doctor           # comprueba dependencias
./target/release/spoted                 # backend social en :7070
```

Sin música en la librería: `spote ingest <carpeta>` o busca con `/` dentro de
la TUI (descarga con yt-dlp).

## Config

`~/.config/spote/config.toml` (todo opcional):

```toml
[server]
url = "https://spote.example.com"
name = "tu-nombre"
invite = "spote"

[ui]
art_mode = "half"   # half | dither | braille | off
viz = "aurora"      # aurora, inferno, void, bloom, glitch, clean, neural
fps = 60
```

## Teclas

`space` play/pausa · `←/→` seek ±5 s (`shift` ±30 s) · `↑/↓` selección ·
`enter` reproducir / unirse a sala · `a` encolar · `f` favorita · `/` buscar ·
`tab` cambiar vista · `t` seguir Spotify · `v` siguiente visualización ·
`V` visualización a pantalla completa · `e` capas · `A` modo de arte ·
`F` portada a pantalla completa · `m` mute · `y` copiar letra · `?` ayuda.

## Documentación

- `idea.md` — producto, arquitectura, decisiones y estado.
- `docs/effects.md` — especificación del port de efectos de `cli-viz`.
- `scripts/frame.py` — captura la TUI en un pty y la convierte en HTML
  (para revisar el diseño sin abrir una terminal).
