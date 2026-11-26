# spote Android

Cliente Android de spote: Kotlin + Jetpack Compose para la piel, Media3 para
reproducir y el core Rust (`crates/core`) a través de UniFFI para biblioteca,
catálogo, letras, paleta y salas.

## Qué funciona

| Pieza | Estado |
| --- | --- |
| Biblioteca local SQLite/FTS5 (`crates/core` vía UniFFI) | ✅ |
| Importar audio del teléfono (SAF + `MediaMetadataRetriever`) | ✅ |
| Reproducción Media3 + `MediaSessionService` + notificación | ✅ |
| Letras: LRCLIB, LyricsPlus (Apple/Spotify/QQ) y respaldo de texto Genius, con tarjeta en el player y vista completa con scroll | ✅ |
| Búsqueda de canciones en el catálogo real (Spotify con respaldo Deezer), no vídeos sueltos | ✅ |
| Fichas de álbum con tracklist completo, año, portada y orden del proveedor | ✅ |
| Perfiles de artista con imagen, seguidores, géneros, top tracks y discografía | ✅ |
| YouTube/SoundCloud on-device (NewPipeExtractor) solo para resolver el audio elegido | ✅ |
| Resolver stream al reproducir (URLs caducan → se resuelven al momento) | ✅ |
| Reproducir HLS de SoundCloud con Media3 | ✅ |
| Descargar una pista remota e indexarla en el SQLite del core conservando su clave canónica | ✅ |
| Descargas con progreso real y cancelación | ✅ |
| Playlists: crear, borrar, añadir y quitar canciones | ✅ |
| Cola: elegir pista, mover arriba/abajo y quitar | ✅ |
| Salas sociales (`spoted`): login, lista, chat, cola y play/pause sincronizado | ✅ |
| Perfil con nombre, foto local y avatar compartido en salas | ✅ |
| QR/enlace `spote://setup` para configurar servidor y entrar solo | ✅ |
| Tema cover-driven con la paleta real del core: la portada tiñe fondo, superficies y acento | ✅ |
| Navegación Spotify: Inicio, Buscar, Biblioteca; mini player sobre las tabs y Sonando a pantalla completa | ✅ |
| Avatar de perfil en la cabecera de las tres tabs, siempre a un toque de Ajustes | ✅ |
| Estilo TUI conmutable: apagado (por defecto), sutil (mono) o medio (mono + scanlines + espectro) | ✅ |
| Búsqueda con historial persistente, caché y limpieza | ✅ |
| Portadas: arte embebido, `art_url` del core y backfill desde iTunes | ✅ |
| Fusión de duplicados con copia de seguridad previa | ✅ |
| Registro local de acciones exportable desde Ajustes | ✅ |
| Actualización in-app desde GitHub Releases | ✅ |

La app usa la librería real por defecto. Si el `.so` no carga, arranca con una
biblioteca vacía (no inventa datos). `spoteMockData=true` cambia a datos mock
para iterar el diseño sin NDK ni dispositivo.

## Navegación y catálogo

- La barra inferior tiene tres tabs: **Inicio**, **Buscar** y **Biblioteca**.
  Al reproducir aparece un mini player encima de las tabs; al tocarlo se abre
  **Sonando** a pantalla completa, sin tabs y con el inset de la barra del
  sistema aplicado una sola vez.
- Las fichas de álbum y artista se cargan del catálogo del core por
  `provider:id`, no a partir de una página de resultados. Si una pista trae un
  álbum que no salió en la búsqueda textual, se abre por nombre con
  `find_album` y se completa el tracklist.
- La búsqueda mixta usa `spotify_api::search_all`: Spotify con respaldo Deezer.
  Devuelve pistas, álbumes y artistas reales; YouTube solo resuelve el audio.
- Las descargas conservan la clave canónica (`isrc:` o `sha1:`) para que la
  copia local y la ficha del catálogo sigan siendo la misma canción.
- La paleta se extrae con el mismo `Palette::from_image` que la TUI: fondo,
  superficies y acento salen de la portada, manteniendo la base oscura de spote.

## Iterar la UI sin emulador

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 GRADLE_USER_HOME=/tmp/spote-gradle \
  ./gradlew :app:recordRoborazziDebug --tests 'dev.spote.app.screenshot.ScreenShotsTest'
```

Las capturas salen en `android/app/build/screenshots/` (01-home, 02-search,
06-now-playing, 15-nocturne-home, …). Roborazzi renderiza Compose en la JVM con
Robolectric: no hace falta emulador y cada captura tarda segundos. Es el bucle
recomendado para agentes y para revisar cambios de diseño. Hay una copia en
`docs/screenshots/android/`.

En Android Studio, abre `android/` y usa **Compose Live Edit** para ver cambios
en el dispositivo sin reinstalar. Los composables tienen `@Preview` donde
interesa.

## Compilar y probar

```bash
./scripts/build-android-lib.sh          # bindings + .so arm64 en jniLibs
# SPOTE_ANDROID_ABIS="arm64-v8a x86_64" ./scripts/build-android-lib.sh
cd android
JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 ./gradlew :app:testDebugUnitTest
JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 ./gradlew :app:assembleDebug
```

El APK queda en `android/app/build/outputs/apk/debug/app-debug.apk`. En un
móvil conectado:

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 ./gradlew :app:installDebug
```

La caché de Gradle de este repo se lleva a `/tmp/spote-gradle` porque `/home`
está justo de espacio; no es obligatorio en otra máquina.

## Resolvedor y descargas on-device

`remote/` usa **NewPipeExtractor v0.27.1** (GPL-3.0-or-later). Busca en YouTube,
elige la mejor pista de audio (M4A/AAC, luego Opus/WebM) y devuelve una URL
directa. `RemoteDownloader` streamea a `.part`, valida el tamaño y renombra con
la extensión correcta; después la app llama a `importLocal` del core para que
quede en la biblioteca.

- `RemoteResolver.init(context)` se llama en `SpoteApplication.onCreate`.
- Las URLs de `googlevideo` caducan y están atadas a la IP: se resuelven justo
  antes de reproducir o descargar. Un 403 pide volver a resolver.
- YouTube puede pedir captcha/`SignInConfirmNotBotException` en redes
  limitadas. La UI lo muestra como error de búsqueda, no rompe la app.
- SoundCloud solo publica HLS en v0.27.1: se puede **buscar y reproducir**
  (Media3 con el módulo HLS), pero no descargar como archivo único; la UI lo
  avisa al intentarlo.
- HLS/DRM no se descargan. `album` suele venir `null`.
- NewPipeExtractor es GPL-3.0; el binario combinado debe distribuirse bajo esos
  términos.

## Salas sociales

En **Ajustes → Salas → Conectar sala** se piden servidor, clave de cliente,
nombre e invitación. El cliente (`social/`) habla con `spoted` por REST y
WebSocket; la sesión se guarda en `EncryptedSharedPreferences` (Keystore). Si
el Keystore no está disponible, cae a almacenamiento privado sin cifrar y la
UI lo advierte.

La home muestra las salas visibles; al entrar se hace `join` y se reciben
estado, cola y los últimos 50 mensajes. El chat y la cola son reales. El
servidor manda el reloj (`anchor_ms`/`start_at_ms`); el cliente calcula el
offset con pings. Para desarrollo local con `http://` hace falta un
`network-security-config` de debug: el manifest usa
`usesCleartextTraffic="false"`.

## Configurar por QR o enlace

El móvil y el desktop comparten el formato:

```text
spote://setup?server=...&key=...&invite=...&name=...
```

- **Móvil**: Ajustes → Salas → Mostrar QR y enlace, o Escanear QR. Al abrir un
  enlace `spote://setup` la app guarda servidor/clave/invitación y hace login
  automático.
- **Desktop**: `spote setup` imprime el QR y el enlace de la config actual;
  `spote setup "<enlace>"` la aplica.
- **Servidor**: `spoted qr <url> <client-key> <invite> [nombre]` genera el QR y
  el enlace para repartir.

El QR contiene la clave de cliente y la invitación: trátalo como una
credencial.

## Portadas y metadatos

Al cargar la biblioteca, las pistas sin `art_path`/`art_url` se rellenan en
background desde la iTunes Search API: se guarda el JPEG en
`filesDir/artwork/`, se actualiza `art_url` en el SQLite del core y, si falta,
se completa el álbum. Las descargas nuevas guardan la miniatura del stream
antes de importar. También se respeta el `art_url` que ya traiga la base de
datos del desktop, así que las bibliotecas antiguas no se quedan sin portada.
La paleta de cada portada se cachea y se usa para teñir la pantalla y el
mini player, igual que hace la TUI.

## Registro de acciones

`ActionLog` escribe JSONL en `filesDir/logs/` (máximo 2 MB por archivo, 7
archivos) con navegación, reproducción, búsquedas, descargas, salas, errores y
tiempos. Las claves con password/token/secret/key/invite se redactan. En
**Ajustes → Diagnóstico** se puede compartir o borrar. No sale del dispositivo
salvo que pulses Compartir.

## Actualizaciones

La app consulta `https://api.github.com/repos/<spoteRepo>/releases/latest`
(como máximo una vez al día), descarga el APK y lanza el instalador del
sistema. Android exige confirmación del usuario: no hay instalación silenciosa.

- `spoteRepo` se configura en `android/gradle.properties` o con
  `-PspoteRepo=owner/name`.
- Los releases deben llamarse `vX.Y.Z` y llevar un asset
  `spote-X.Y.Z-arm64-v8a.apk` (o `universal`).
- El workflow `.github/workflows/android-release.yml` compila, firma y publica.
  Secretos: `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`.
- Si el usuario no ha dado permiso de instalar apps desconocidas, “Instalar”
  abre los ajustes del sistema; al volver, se pulsa otra vez.

## Rendimiento

- La biblioteca SQLite se lee en `Dispatchers.IO`; la UI arranca con un estado
  vacío y se rellena sin bloquear el primer frame.
- Navegar a álbum/artista/playlist usa los objetos ya cargados en memoria, sin
  consultar SQLite durante la composición.
- La cola de Media3 se cachea y solo se reconstruye cuando cambia el timeline;
  el tick de posición no vuelve a mapear cientos de pistas.
- La búsqueda local va con debounce y la remota se cachea por consulta.

## Builds

- AGP 8.13.2 · Gradle 8.14.8 · Kotlin 2.3.21 · compileSdk 36 · minSdk 26.
- `crates/android` compila el core con `default-features = false`; MPRIS/zbus
  queda fuera. Los perfiles `android-release`/`android-dev` usan
  `panic = "unwind"` para que UniFFI pueda convertir panics.
- Las bindings Kotlin (`app/src/main/java/dev/spote/rust/spote_android.kt`) son
  generadas: no se editan a mano. Se regeneran con `build-android-lib.sh`.
- `jniLibs/<abi>/libspote_android.so` también es generado.
- 16 KB page size: NDK r28 y cargo-ndk 4.x ya lo ponen; JNA 5.19.1 lo soporta.

## Pendiente

- Reanudar descargas tras matar el proceso (ahora viven en el ViewModel; el
  `.part` se borra al cancelar).
- Importar playlists de Spotify y reordenar canciones dentro de una playlist.
- ASR local para letras (requiere runtime whisper.cpp y un modelo en el APK).
- Alinear en el tiempo la letra plana de Genius (ahora se muestra sin tiempos).
- `MediaProjection` para grabar el monitor.
- Resolver de golpe toda una cola remota en vez de pista a pista.
