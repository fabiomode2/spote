# spoted — backend social

`spoted` es el servidor de salas de spote: presencia, chat y cola compartida.
No toca audio, no descarga nada y no sirve ficheros: solo coordina SQLite y
WebSocket. Está pensado para un grupo pequeño, pero la puerta está cerrada con
clave de cliente y sesiones atadas a esa clave.

```
TUI / cliente ──HTTPS/WSS──▶ nginx ──HTTP──▶ spoted (contenedor) ──▶ SQLite
                              │
                              └─ X-Spote-Key obligatoria en /api/v1
```

## Modelo de seguridad

Hay dos capas independientes:

1. **Clave de cliente (`X-Spote-Key`)**: se configura en cada cliente y se
   comprueba en *todas* las rutas de `/api/v1`, incluidas las que no existen y
   el handshake del WebSocket. Sin una clave válida la API responde `401` y no
   revela si una ruta existe. `/healthz` es la única excepción y solo devuelve
   un latido.
2. **Sesión (`Authorization: Bearer …`)**: se obtiene con
   `POST /api/v1/auth/login` y el código de invitación. El token:
   - se guarda **hasheado** en SQLite (un backup no sirve para robar sesiones);
   - está atado a la huella de la clave de cliente que lo creó;
   - caduca (`SPOTE_SESSION_DAYS`, 30 días por defecto);
   - se puede revocar con `POST /api/v1/auth/logout`.

Además, el nombre de usuario queda atado a la primera clave de cliente que hace
login con él. Una clave distinta no puede reclamar el mismo nombre aunque tenga
la invitación. Cada amigo debe tener **su propia clave**, no una compartida.

Otras medidas:

- La invitación y las claves se comparan por SHA-256 en tiempo constante.
- No hay valores por defecto para los secretos: si faltan, `spoted` no arranca.
- Los tokens se generan con 32 bytes aleatorios y nunca se escriben en logs.
- El token no viaja por query string; el WebSocket usa la cabecera
  `Authorization`, así no acaba en logs de nginx ni en el historial.
- CORS está cerrado salvo que se configure `SPOTE_CORS_ORIGIN`.
- Límite de cuerpo HTTP: 1 MiB (sync lleva lotes); frames WebSocket: 64 KiB.
  El push de sync admite como máximo 250 operaciones por lote.
- Validación de texto: se rechazan caracteres de control (ANSI, `\r`, etc.) en
  nombres, chat, salas y metadatos de pista.
- Salas privadas: solo el host añade miembros (`POST /rooms/{id}/members`).
- Cola: solo el dueño del item o el host pueden quitarlo.
- Sync: cada fila está filtrada por `user_id`; un usuario no puede leer ni
  escribir la biblioteca de otro cambiando un id. Los pushes son atómicos y
  chocan contra la revisión base si otro dispositivo escribió antes.
- Límites por conexión para chat, cola, controles y presencia; máximo de
  conexiones WebSocket por usuario y en total.
- SQLite en WAL, foreign keys activadas, consultas parametrizadas y errores
  internos genéricos hacia el cliente.

## Configuración

| variable | obligatoria | descripción |
| --- | --- | --- |
| `SPOTE_INVITE` | sí | invitación compartida del grupo; mínimo 16 caracteres |
| `SPOTE_CLIENT_KEYS` | sí | claves por cliente (mín. 32 caracteres), formato `etiqueta:clave,etiqueta:clave` |
| `SPOTE_BIND` | no | `127.0.0.1:7070` por defecto |
| `SPOTE_DB` | no | `./spote-server.db` por defecto |
| `SPOTE_SESSION_DAYS` | no | días de validez de una sesión (1–3650, 30 por defecto) |
| `SPOTE_CORS_ORIGIN` | no | origen exacto permitido por CORS |
| `SPOTE_INVITE_FILE` | no | lee la invitación de un fichero (Docker secrets) |
| `SPOTE_CLIENT_KEYS_FILE` | no | lee las claves de un fichero (Docker secrets) |

Las variables directas tienen prioridad sobre `*_FILE`. Los ficheros se leen
una vez al arrancar. El arranque rechaza secretos con patrones obvios (menos de
6 caracteres distintos) y el valor de ejemplo de la invitación. Formato de
`SPOTE_CLIENT_KEYS`:

```
ana:0123…64-hex,bea:4567…64-hex
```

Genera secretos sin copiar de internet:

```bash
cargo run -p spote-server -- keygen ana bea
# o, dentro de la imagen:
docker compose run --rm spoted keygen ana bea
```

Guarda el `.env` con permisos `600`. Si usas Docker secrets, monta los ficheros
en `/run/secrets/…` y usa `SPOTE_*_FILE`; así las claves no aparecen en
`docker inspect`.

## Cuenta y dispositivos

La cuenta es el nombre de usuario + tu `client_key`. El primer login con un
nombre crea la cuenta y la ata a esa clave; no hay un registro separado. Para
que el PC y el móvil sean la misma cuenta, los dos deben usar **la misma clave
personal**; no es una clave por dispositivo. No la compartas con otra persona:
quien la tenga puede entrar como tú. En Android, guarda la clave y el token en
el almacén seguro del sistema (Keystore), no los incrustes en el APK.

El servidor guarda una biblioteca personal por cuenta:

- metadatos de pistas (título, artista, álbum, duración, carátula);
- favoritas, `play_count` y `last_played`;
- playlists y sus items ordenados.

No sincroniza audio, rutas locales ni descargas: eso se queda en cada
dispositivo. Una pista local del PC aparecerá en el móvil como metadatos, pero
el móvil tendrá que resolver su propia fuente si quiere sonar.

Login desde cualquier cliente:

```bash
curl -X POST https://spote.example.com/api/v1/auth/login \
  -H "X-Spote-Key: $CLIENT_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name":"ana","invite":"…"}'
```

La respuesta trae `token`; se manda como `Authorization: Bearer …` en todo lo
demás. Android debe guardar `client_key`, `invite` y `token` en Keystore.

### Sync incremental

Cada cuenta tiene un contador `revision` que solo sube. El cliente guarda el
último que conoce y pide cambios a partir de él. Los borrados son tombstones
para que el otro dispositivo sepa que debe quitarlos.

```
GET  /api/v1/sync/snapshot                    → estado completo activo
GET  /api/v1/sync/pull?since=<revision>       → cambios desde esa revisión
POST /api/v1/sync/push {base_revision, ops}   → {revision, applied}
```

`push` es atómico. Si `base_revision` no es la revisión actual, responde `409` y
no aplica nada: el cliente hace `pull`, mezcla y reintenta. Si un cliente pide
`pull` con una revisión mayor que la del servidor, recibe `400`: vuelve a hacer
`snapshot`. Los `ops` son:

```json
{"op":"track_upsert","track":{"key":"isrc:…","title":"…","artist":"…"},
 "favorite":true,"play_count":3,"last_played":1700000000000}
{"op":"track_delete","key":"isrc:…"}
{"op":"playlist_upsert","id":"pl-1","name":"Favoritas",
 "description":"","cover_url":"https://…"}
{"op":"playlist_delete","id":"pl-1"}
{"op":"playlist_item_upsert","id":"item-1","playlist_id":"pl-1",
 "track_key":"isrc:…","position":0}
{"op":"playlist_item_delete","id":"item-1"}
```

Los ids de playlist/item los genera el cliente (`[A-Za-z0-9_-]`, máx. 64) para
que dos dispositivos no necesiten traducir ids locales. Un push admite hasta
250 operaciones; parte la biblioteca en lotes si hace falta.

Flujo recomendado en cada dispositivo:

1. primer arranque: `sync/snapshot`, guardar `revision`;
2. al cambiar favoritas/playlists: `sync/push` con esa `base_revision`;
3. si llega `409`: `sync/pull?since=…`, aplicar cambios y reintentar;
4. cada pocos minutos: `sync/pull?since=revision` para traer lo del otro.

Para Android: guarda `base_url`, `client_key`, `invite`, `name` y `token` en
DataStore/Keystore; enseña un formulario de ajustes para los cuatro primeros.
`Track.key` es la clave estable de canción, `Playlist.id` e item ids son strings
que genera el cliente, y `Track.downloaded`/rutas locales son cosa de cada
dispositivo (no viajan).

## Cliente

En `~/.config/spote/config.toml`:

```toml
[server]
url = "https://spote.example.com"
name = "ana"
invite = "…"                 # el mismo para todos
client_key = "…"             # la clave personal de ana
token = ""                   # se rellena solo tras el login
```

El token se guarda tras el primer login. Si caduca o la clave cambia, la TUI
renueva la sesión al arrancar; si el nombre ya está atado a otra clave, el
login se rechaza. En ese caso, revisa `client_key` o pide un reset al host.

## Docker

```bash
cp .env.example .env
chmod 600 .env
# edita .env con keygen
docker compose up -d --build
docker compose logs -f spoted
```

El contenedor:

- corre como UID 10001, sin privilegios (`cap_drop: ALL`,
  `no-new-privileges`);
- tiene el sistema de ficheros de solo lectura, con `/data` en un volumen y
  `/tmp` en tmpfs;
- escucha solo en `127.0.0.1:7070` del host;
- guarda SQLite en el volumen `spote-data`.

Si cambias el volumen por un bind mount, dale permisos al UID del contenedor:
`chown -R 10001:10001 ./data`.

No publiques el puerto 7070. En el firewall del VPS abre solo 80/443 (por
ejemplo `ufw allow 'Nginx Full'`) y deja 7070 en loopback. Si nginx está en
otro contenedor, quita el `ports` y mételo en la misma red; el `proxy_pass`
apuntará a `http://spoted:7070`.

Al actualizar desde una versión anterior sin claves de cliente, las sesiones
viejas se invalidan al migrar la base de datos. Cada cliente vuelve a hacer
login una vez con su `client_key`; el nombre se ata a esa clave en ese momento.

## nginx

`deploy/nginx/spote.conf` es una plantilla completa:

- TLS 1.2/1.3, HSTS y redirección de HTTP a HTTPS;
- `client_max_body_size 64k`;
- límite de peticiones y conexiones por IP;
- `limit_req` en `/api/`;
- WebSocket con `Upgrade`/`Connection`;
- corte temprano si falta `X-Spote-Key` (spoted lo vuelve a comprobar);
- `/healthz` sin autenticación y sin datos;
- todo lo que no sea `/api/` o `/healthz` devuelve 404.

Copia el fichero, cambia el dominio y los certificados, y valida con
`nginx -t`. Si usas Cloudflare u otro proxy delante, no confíes en
`X-Forwarded-For` para límites de seguridad sin configurarlo explícitamente.

## API

Todas las rutas salvo `/healthz` exigen `X-Spote-Key`. Las rutas de usuario
exigen además `Authorization: Bearer <token>`. Ejemplo mínimo:

```bash
KEY="$(grep SPOTE_CLIENT_KEYS .env | sed 's/.*ana://; s/,.*//')"
curl -H "X-Spote-Key: $KEY" https://spote.example.com/api/v1/time
curl -H "X-Spote-Key: $KEY" https://spote.example.com/api/v1/me \
     -H "Authorization: Bearer $TOKEN"
```

```
POST /api/v1/auth/login      {name, invite}        → {token, user, invite}
POST /api/v1/auth/logout                            → {ok}
GET  /api/v1/me                                     → user
GET  /api/v1/users                                  → presencia
POST /api/v1/friends/{id}
GET  /api/v1/sync/snapshot                          → biblioteca personal
GET  /api/v1/sync/pull?since=…                      → cambios desde revisión
POST /api/v1/sync/push {base_revision, ops}         → {revision, applied}
GET  /api/v1/rooms                                  → salas visibles
POST /api/v1/rooms           {name, is_public}      → room
GET  /api/v1/rooms/{id}                             → snapshot
POST /api/v1/rooms/{id}/join | /leave
POST /api/v1/rooms/{id}/members {user_id}           → solo host
DEL  /api/v1/rooms/{id}/members/{user}              → solo host
POST /api/v1/rooms/{id}/control {action, position_ms?}
POST /api/v1/rooms/{id}/queue   {track: TrackRef}   → queue_item
DEL  /api/v1/rooms/{id}/queue/{item}                → dueño o host
GET  /api/v1/rooms/{id}/chat?limit=50
GET  /api/v1/time                                   → {t} (reloj)
GET  /api/v1/ws                                     → WebSocket
GET  /healthz                                       → {ok, version}
```

El WebSocket usa `Authorization: Bearer …` y `X-Spote-Key` en el handshake.
Mensajes cliente→servidor: `hello`, `ping`, `join`, `leave`, `control`,
`queue_add`, `queue_remove`, `ready`, `presence`, `chat`. Servidor→cliente:
`welcome`, `pong`, `room_state`, `queue`, `start_at`, `presence`, `chat`,
`error`.

## Operación

### Rotar o resetear una clave

Las claves se cambian en `SPOTE_CLIENT_KEYS` y se reinicia el contenedor. Los
usuarios atados a la clave vieja dejan de poder entrar. Para volver a atar un
nombre a la clave nueva, con el servidor parado:

```bash
docker compose stop spoted
docker compose run --rm --entrypoint sqlite3 spoted /data/spote-server.db \
  "UPDATE users SET client_key_fp = NULL WHERE name = 'ana';
   DELETE FROM sessions WHERE user_id = (SELECT id FROM users WHERE name = 'ana');"
docker compose start spoted
```

La idea es: `client_key_fp = NULL`, borrar sesiones y reiniciar; el siguiente
login con la clave nueva vuelve a atar el nombre. La imagen incluye `sqlite3`
justo para estas operaciones.

### Copias de seguridad

SQLite en WAL. Con el contenedor parado (o usando `.backup`) copia el volumen:

```bash
docker compose stop spoted
docker run --rm -v spote-data:/data -v "$PWD/backups":/backup debian:bookworm-slim \
  sh -c 'tar czf /backup/spote-$(date +%F).tgz -C /data .'
docker compose start spoted
```

Guarda también el `.env` o los secretos: sin la invitación y las claves no se
puede volver a entrar, aunque la base de datos esté intacta.

### Logs y salud

```bash
curl -fsS http://127.0.0.1:7070/healthz
docker compose ps
docker compose logs --tail=200 spoted
```

Los logs no incluyen claves, tokens ni query strings. Si ves un `401` en nginx,
comprueba que el cliente manda `X-Spote-Key`; si es el login, revisa invitación
y que el nombre no esté atado a otra clave.

## Límites conocidos

- SQLite y una única conexión protegida por mutex: suficiente para un grupo
  pequeño, no para miles de usuarios concurrentes.
- La presencia se difunde a todos los usuarios autenticados; no hay lista de
  amigos filtrada por privacidad.
- El host no puede borrar una sala desde la API todavía; se puede hacer por
  SQLite o añadir el endpoint.
- La TUI aún no tiene formulario para crear salas privadas ni para añadir
  miembros; se hace por REST.
