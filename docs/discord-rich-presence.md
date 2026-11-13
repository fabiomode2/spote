# Discord Rich Presence — puesta en marcha

spote puede publicar en tu perfil de Discord lo que está sonando: título,
`artista · álbum`, portada, tiempo transcurrido y barra de progreso, y un botón
**Abrir en Spotify** cuando la pista es de Spotify.

Funciona con spote reproduciendo su propia música y también en
`spote --follow-spotify` (cuando escuchas en el cliente oficial y spote sólo
acompaña).

---

## 1. Por qué hace falta un Application ID

Discord no permite publicar presencia "anónima". El handshake del protocolo RPC
es:

```json
{"v": 1, "client_id": "<Application ID>"}
```

y Discord valida ese ID contra su registro de aplicaciones. Con uno inventado
cierra la conexión con:

```json
{"code": 4000, "message": "Invalid Client ID"}
```

Comprobado en vivo contra el cliente de este equipo: el socket existe, el
handshake se envía bien y Discord contesta exactamente ese error. **No hay modo
sin ID**, pero crearlo es gratis, instantáneo y no pasa revisión.

El nombre y los iconos que se ven en el perfil salen de esa aplicación, así que
además es el sitio correcto para poner el logo de spote cuando exista.

---

## 2. Los tres pasos (2 minutos)

1. Entra en <https://discord.com/developers/applications> con tu cuenta.
2. **New Application** → nombre `spote` → *Create*. No hace falta bot, ni
   permisos, ni servidor, ni verificación.
3. En **General Information**, copia el **Application ID** (un número largo; no
   es el *Client Secret*, no lo confundas).

Y guárdalo:

```bash
spote discord --id <APPLICATION_ID> --save
```

Eso escribe el ID en `~/.config/spote/config.toml` y prueba la conexión al
momento. Si Discord está abierto, verás la actividad en tu perfil durante unos
segundos; si contesta `Invalid Client ID`, el número está mal copiado.

También puedes editarlo a mano:

```toml
# ~/.config/spote/config.toml
[discord]
enabled = true
client_id = "1234567890123456789"
buttons = true
small_image = "spote"
```

---

## 3. Comprobar que funciona

```bash
spote discord --id <ID>              # manda la pista actual (o una de prueba) y la mantiene 12 s
spote discord --id <ID> --seconds 30 # lo mismo, más tiempo
```

Después, abre la TUI (o arranca la reproducción): la presencia se actualiza
sola. Mira tu propio perfil (o pide a alguien que lo mire) mientras suena algo.

Detalles de lo que se envía:

| Campo | Contenido |
|---|---|
| `details` | título de la pista |
| `state` | `artista · álbum` |
| `timestamps.start` | momento en que empezó (Discord cuenta el tiempo solo) |
| `timestamps.end` | final estimado → barra de progreso |
| `assets.large_image` | portada de Spotify por URL (no hay que subirla) |
| `assets.large_text` | álbum |
| `assets.small_image` | asset `spote` de la app (el logo, cuando esté subido) |
| `buttons` | *Abrir en Spotify* |

Al pausar, la actividad se borra. Al saltar de pista o mover la posición más de
2 s, se recalcula el tiempo. Límite de envíos: uno cada 2 s (Discord permite ~5
cada 20 s).

---

## 4. El logo

1. En la app de Discord: **Rich Presence → Art Assets**.
2. Sube el PNG (mínimo 512×512, cuadrado) con el nombre **`spote`**.
3. Ya está: la config trae `small_image = "spote"`, así que aparece solo. Si el
   asset no existe todavía, Discord simplemente no pinta nada en ese hueco.

Si algún día quieres que el logo sea el icono grande en vez del pequeño, cambia
`small_image` por `large_image` en `crates/tui/src/spotify_jobs.rs` (o añade una
opción a la config).

---

## 5. Problemas típicos

| Síntoma | Causa / solución |
|---|---|
| `no encuentro Discord abierto` | Discord cerrado, o instalado de forma rara. spote busca en `$XDG_RUNTIME_DIR`, `$TMPDIR` y `/tmp`, incluidas las rutas de Flatpak (`app/com.discordapp.Discord`) y Snap. Si usas Flatpak, comprueba que el socket esté en `$XDG_RUNTIME_DIR/app/com.discordapp.Discord/`. |
| `Invalid Client ID` | ID mal copiado, o es el *Client Secret*. Es un número, no un token. |
| No aparece nada | `[discord] enabled = false`, `client_id` vacío, o la app no está corriendo (la actividad la publica spote, no Discord). |
| Se queda con una canción vieja | Discord cerró el socket (reinicio del cliente): spote reconecta en la siguiente actualización. Si insiste, cierra y abre spote. |
| No se actualiza al cambiar de pista | Comprueba que `enabled = true` y que el ID sigue ahí tras un `save()` de la config. |
| Sale otro nombre (no "spote") | El perfil enseña el nombre de la **aplicación de Discord**, no el del binario. Se cambia en el portal. |

---

## 6. Dónde está implementado

| Sitio | Qué hace |
|---|---|
| `crates/core/src/discord.rs` | Cliente IPC (descubrimiento de socket, framing, handshake, `SET_ACTIVITY`, limpieza), `Activity` → JSON, worker con reconexión y rate limit. |
| `crates/tui/src/spotify_jobs.rs` → `DiscordPresence` | Traduce `NowPlaying` a `Activity`: título, artista·álbum, timestamps, portada, botón. |
| `crates/tui/src/app.rs` | Campo `discord`, inicialización y `poll_spotify_jobs()` en el tick (6 líneas en total). |
| `crates/core/src/config.rs` | Sección `[discord]`: `enabled`, `client_id`, `buttons`, `small_image`. |
| CLI | `spote discord --id <id> [--save] [--seconds N]`. |

Sin dependencias nuevas: socket Unix de la stdlib y `serde_json`.
