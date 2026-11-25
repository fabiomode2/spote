# Actualizaciones in-app

Updater autocontenido para builds de GitHub Releases. Usa sólo
`HttpURLConnection`, `org.json` y Compose/Material 3 (sin dependencias nuevas).

## Integración

La pantalla de Ajustes ya expone un slot `updateContent`; basta con sustituir
el placeholder por la tarjeta:

```kotlin
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.spote.app.ui.update.UpdateCard

SettingsScreen(
    onBack = { navController.popBackStack() },
    onOpenDownloads = { navController.navigate("downloads") },
    updateContent = {
        UpdateCard(
            viewModel = viewModel(),
            modifier = Modifier.padding(horizontal = 20.dp, vertical = 6.dp),
        )
    },
)
```

Fuera de ese slot, el uso es el mismo:

```kotlin
UpdateCard(
    viewModel = viewModel(),
    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
)
```

`UpdateViewModel` es un `AndroidViewModel` con constructor `(Application)`:
`viewModel()` lo crea con la factory por defecto, **no hace falta factory
personalizada**.

Al crearse lanza una comprobación automática. Esa comprobación está limitada a
una cada 24 h mediante `SharedPreferences` (`spote_updates`). El botón "Buscar"
llama a `check(force = true)` e ignora ese límite.

Flujo:

1. `check` consulta `https://api.github.com/repos/<GITHUB_REPO>/releases/latest`.
2. Si hay una versión semver mayor, muestra notas y el botón "Descargar".
3. `download` guarda el APK en `filesDir/updates/` e informa del progreso. Si
   la release trae `digest: sha256:…`, se verifica antes de dar el APK por
   bueno; cualquier fallo borra el `.part`.
4. `install` abre el instalador del sistema. Nunca instala en silencio.

## Permisos y FileProvider

Ya están declarados en `AndroidManifest.xml`; no hay que añadir nada:

- `INTERNET` (consulta y descarga).
- `REQUEST_INSTALL_PACKAGES` (abrir el instalador).
- `FileProvider` con autoridad `${applicationId}.fileprovider` y
  `@xml/file_paths` apuntando a `files/updates`.

La autoridad real se toma de `context.packageName`, así que en la variante
debug (`applicationIdSuffix = ".debug"`) es
`dev.spote.app.debug.fileprovider`. No hardcodear la autoridad.

La primera vez, Android puede denegar la instalación: `UpdateManager.install`
detecta `canRequestPackageInstalls() == false`, abre
`Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES` para el paquete y deja el estado
en `Ready`. Al volver, el usuario pulsa "Instalar" otra vez.

## Formato de las releases

- Tag: `vX.Y.Z` (por ejemplo `v0.2.0`), **sin** marcar como pre-release.
  `latest` ignora borradores y releases marcadas `prerelease`.
- Asset APK: `spote-X.Y.Z-arm64-v8a.apk`. El cliente elige el primer `.apk`
  cuyo nombre contenga `arm64` o `universal`; si no hay ninguno, el primero.
- Versión y repo vienen de `BuildConfig.GITHUB_REPO` y
  `BuildConfig.VERSION_NAME` (por defecto `fabiomode2/spote`; se puede
  sobreescribir con `-PspoteRepo=owner/name`).
- El `versionCode` que genera CI es `major * 10000 + minor * 100 + patch`
  (por ejemplo `0.2.0` → `200`).

El workflow `.github/workflows/android-release.yml` compila el APK, lo renombra
a `spote-X.Y.Z-arm64-v8a.apk` y lo adjunta a la release del tag. Un
`workflow_dispatch` compila la versión indicada pero sube el APK como artefacto
en vez de publicar una release.

## Notas

- `RELEASE` se firma con las props `-PspoteKeystore`, `-PspoteKeystorePassword`,
  `-PspoteKeyAlias` y `-PspoteKeyPassword` que ya lee `app/build.gradle.kts`.
- La comparación semver tolera prefijo `v`, sufijo `-debug`, versiones parciales
  (`1.2`) y metadatos (`+build`); los sufijos de pre-release se ordenan por
  debajo de la versión estable.
- Tests: `UpdateVersionTest` cubre comparación semver, elección de asset y
  parseo del digest. No toca APIs de Android.
