package dev.spote.app.update

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.spote.app.BuildConfig
import java.io.File
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Orquesta la comprobación, descarga e instalación del APK.
 *
 * Es un [AndroidViewModel] con constructor `(Application)`, así que
 * `viewModel()` puede crearlo sin factory personalizada.
 */
class UpdateViewModel(application: Application) : AndroidViewModel(application) {

    private val manager = UpdateManager(
        context = application,
        repo = BuildConfig.GITHUB_REPO,
        currentVersion = BuildConfig.VERSION_NAME,
    )

    private val _state = MutableStateFlow<UpdateUiState>(UpdateUiState.Idle)
    val state: StateFlow<UpdateUiState> = _state.asStateFlow()

    private var pendingRelease: ReleaseInfo? = null
    private var downloadedApk: File? = null
    private var operation: Job? = null

    init {
        check(force = false)
    }

    fun check(force: Boolean = false) {
        if (operation?.isActive == true) return

        operation = viewModelScope.launch {
            _state.value = UpdateUiState.Checking
            try {
                when (val result = manager.check(force)) {
                    UpdateCheckResult.Throttled -> {
                        _state.value = UpdateUiState.Idle
                    }

                    UpdateCheckResult.NoRelease -> {
                        _state.value = UpdateUiState.UpToDate(BuildConfig.VERSION_NAME)
                    }

                    is UpdateCheckResult.UpToDate -> {
                        _state.value = UpdateUiState.UpToDate(result.currentVersion)
                    }

                    is UpdateCheckResult.Available -> {
                        pendingRelease = result.release
                        downloadedApk = null
                        _state.value = UpdateUiState.Available(
                            current = BuildConfig.VERSION_NAME,
                            latest = result.release.version,
                            notes = result.release.notes,
                            apkUrl = result.release.apkUrl,
                            sha256 = result.release.sha256,
                        )
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.value = UpdateUiState.Error(
                    e.toUserMessage("No se pudo comprobar la actualización"),
                )
            }
        }
    }

    fun download() {
        val release = pendingRelease ?: return
        if (operation?.isActive == true) return

        operation = viewModelScope.launch {
            _state.value = UpdateUiState.Downloading(progress = 0f, latest = release.version)
            try {
                val apk = manager.download(release) { progress ->
                    _state.value = UpdateUiState.Downloading(
                        progress = progress.coerceIn(0f, 1f),
                        latest = release.version,
                    )
                }
                downloadedApk = apk
                _state.value = UpdateUiState.Ready(release.version, apk.absolutePath)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.value = UpdateUiState.Error(
                    e.toUserMessage("No se pudo descargar la actualización"),
                )
            }
        }
    }

    fun install() {
        val apk = downloadedApk
            ?: (_state.value as? UpdateUiState.Ready)?.let { File(it.apkPath) }
        if (apk == null) {
            _state.value = UpdateUiState.Error("No hay ningún APK descargado")
            return
        }

        try {
            manager.install(apk)
        } catch (e: Exception) {
            _state.value = UpdateUiState.Error(
                e.toUserMessage("No se pudo abrir el instalador"),
            )
        }
    }

    fun dismiss() {
        operation?.cancel()
        operation = null
        pendingRelease = null
        downloadedApk = null
        _state.value = UpdateUiState.Idle
    }

    private fun Throwable.toUserMessage(fallback: String): String = when (this) {
        is UnknownHostException -> "Sin conexión a internet"
        is SocketTimeoutException -> "La conexión tardó demasiado"
        else -> message?.takeIf { it.isNotBlank() } ?: fallback
    }
}
