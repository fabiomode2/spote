package dev.spote.app.update

/**
 * Datos de la release más reciente publicada en GitHub.
 *
 * [version] es el tag sin la "v" inicial. [sha256] sólo se rellena si GitHub
 * expone el campo `digest` del asset (formato `sha256:...`).
 */
data class ReleaseInfo(
    val tag: String,
    val version: String,
    val notes: String,
    val apkUrl: String,
    val apkName: String,
    val publishedAt: String,
    val sha256: String?,
)

/** Estado que consume la tarjeta de actualizaciones. */
sealed interface UpdateUiState {
    data object Idle : UpdateUiState

    data object Checking : UpdateUiState

    data class UpToDate(val current: String) : UpdateUiState

    data class Available(
        val current: String,
        val latest: String,
        val notes: String,
        val apkUrl: String,
        val sha256: String?,
    ) : UpdateUiState

    data class Downloading(val progress: Float, val latest: String) : UpdateUiState

    data class Ready(val latest: String, val apkPath: String) : UpdateUiState

    data class Error(val message: String) : UpdateUiState
}
