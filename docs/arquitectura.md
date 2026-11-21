# Arquitectura de spote

Mapa de módulos y reglas para añadir cosas sin romper el resto. La idea: la UI
solo lee estado y escribe acciones; todo lo que bloquea (red, disco, yt-dlp,
MPRIS, SQLite) vive en un worker.

## Crates

| crate | papel |
| --- | --- |
| `crates/protocol` | tipos compartidos (`TrackRef`, salas, cola). Sin I/O. |
| `crates/core` | biblioteca SQLite, descargas, catálogo, letras, MPRIS, arte, resolución de streaming y política de cola. |
| `crates/server` | `spoted`: presencia, salas y cola ordenada por servidor. |
| `crates/tui` | binario `spote`: estado de la app y dibujo en el terminal. |

## TUI: `app/` (estado y efectos)

| módulo | contiene |
| --- | --- |
| `app/mod.rs` | `App`, enums (`Tab`, `Action`, `Overlay`...), arranque, `tick`, reproducción, cola, búsqueda, salas, arte. |
| `app/input.rs` | teclado (`on_key`). |
| `app/actions.rs` | ratón y `perform(Action)` (lo que dispara un clic o una tecla ya traducida). |
| `app/settings.rs` | `SettingRow`, valores, `adjust_setting`/`toggle_setting`, guardado. |
| `app/downloads.rs` | estado de descargas y su progreso. |
| `app/playlists.rs` | formularios de listas, crear/borrar/añadir. |
| `app/spotify.rs` | modo acompañante por MPRIS. |
| `app/albums.rs` | página de álbum (biblioteca o catálogo), tracklist, descarga completa y guardar como lista. |
| `app/artist.rs` | perfil de artista (Spotify/Deezer) y backfill de portada/metadatos. |
| `app/context.rs` | menú contextual de clic derecho (estado, borrar, descargar, artista). |
| `app/layout_editor.rs` | editor de diseño en caliente (`E`): paneles, docks y tamaños. |
| `app/keymap.rs` | fuente única de leyendas y ayuda; la barra inferior no miente. |

Cada frame: `tick()` → sondea workers (`poll_stream`, `poll_downloads`,
`poll_art`, `poll_lyrics`, `poll_spotify`, `poll_social`, `poll_spotify_jobs`)
→ `ui::draw`
lee el estado → el usuario pulsa algo → `on_key`/`on_mouse` → `perform`.

Los workers (`fetch.rs`, `spotify_jobs.rs`, `Downloader`, `stream.rs`) hablan por canales
`std::sync::mpsc`; nunca se llama a red/disco desde el hilo de dibujo.

## TUI: `ui/` (dibujo)

| módulo | contiene |
| --- | --- |
| `ui/layout.rs` | layout declarativo: `LayoutCfg` → rectángulos, docks, centrado vertical, asas de arrastre y proporción de portada. |
| `ui/mod.rs` | `draw`, fondo ambiental (void/neural/glitch/cover/lyrics), primitivas (`put`, `fill`, `panel`, `text_at`, `centered`, `truncate_chars`). |
| `ui/albums.rs` | tarjetas de álbum (1-2 columnas) y detalle con portada. |
| `hires.rs` | portada PNG real en Kitty (protocolo gráfico, solo cuando cambia). |
| `ui/art.rs` | portada grande, marco, miniaturas y badges. |
| `ui/now.rs` | vista «ahora»: cabecera, letra palabra a palabra, clic para seek y tamaño de letra. |
| `ui/lists.rs` | búsqueda mixta (artistas/álbumes/pistas), biblioteca, tarjetas y salas. |
| `ui/side.rs` | cola, amigos/feeling de la canción y visualizador. |
| `ui/settings.rs` | pestaña de ajustes. |
| `ui/chrome.rs` | barra superior, navegación, transporte, barra de progreso y leyenda. |
| `ui/overlays.rs` | ayuda, editor de efectos y formularios modales. |

Todo lo clickable se registra en `app.hit` (`Rect → Action`). El último registro
gana, así que un botón pequeño debe añadirse **después** de su fila.

### Añadir una vista

1. `Tab::ALL` y los `match` de `Tab` (label, icon, index, next/prev) en
   `app/mod.rs`.
2. Un `pub(super) fn draw_x(...)` en el módulo de `ui/` que corresponda y una
   rama en `ui::draw`.
3. `X = 1 + 1` en el `match app.tab` de `refresh_view` si tiene lista propia.
4. Sus teclas en `app/input.rs` y sus clics como `Action` en `app/actions.rs`.

### Añadir un ajuste

1. Campo en `UiCfg` (`crates/core/src/config.rs`) con default.
2. Fila en `SettingRow::ALL` + `label()` + `setting_value()`.
3. Si se toca con flechas, añadirlo a `SettingRow::adjustable` y a
   `adjust_setting`; si es un sí/no, a `toggle_setting`.
4. Copiar el campo a `App` en `App::new` y persistirlo con `cfg.save()`.
5. La UI de ajustes se genera sola con `settings_rows()`.

### Añadir un overlay

1. Variante en `Overlay` y estado en `App` si hay formulario.
2. Rama en `handle_overlay_key` (`app/playlists.rs` o `app/input.rs`).
3. `draw_*` en `ui/overlays.rs` y rama en `draw_overlay`.
4. `overlay_frame()` centra y pinta el marco; no dupliques ese código.

### Añadir un trabajo de fondo

1. Evento en el enum del worker (`JobEvent`/`DownloadEvent`).
2. `std::thread::spawn` con el `Sender` clonado; nunca bloquear `tick`.
3. `poll_*` en `app` aplica el evento al estado y hace `toast` si procede.

## Reglas

- Un frame no bloquea: presupuesto ~16 ms a 60 fps (los efectos tienen su
  propio reloj, más lento). Nada de `reqwest`, `yt-dlp` o MPRIS en el hilo de
  dibujo. Los contadores y listas de SQLite que se pintan cada frame pasan por
  una caché corta en `Library`.
- El texto de usuario se recorta con `truncate_chars`, nunca con
  `String::truncate` (revienta con acentos y emoji).
- La portada manda: la paleta sale del arte y los colores de la UI se derivan
  de ella; `plain` es la única excepción (foto literal).
- Los ajustes se guardan solos (`cfg.save()` tras cada cambio).
- Tests: lógica en `core` (sin red), layout y helpers en `ui`, y un test por
  cada parser nuevo. `cargo test --workspace` antes de commitear.
