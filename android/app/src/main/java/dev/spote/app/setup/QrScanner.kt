package dev.spote.app.setup

import android.app.Activity
import android.content.Intent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext

private const val DEFAULT_SCAN_PROMPT = "Apunta al QR de configuración de Spote"

/**
 * Abre el escáner CameraX + ML Kit y entrega el texto leído.
 *
 * Si el usuario cancela no se llama a [onResult]. La Activity de captura está
 * declarada en el manifest y pide el permiso de cámara en el momento.
 */
@Composable
fun rememberQrScanner(
    onResult: (String) -> Unit,
    prompt: String = DEFAULT_SCAN_PROMPT,
): () -> Unit {
    val context = LocalContext.current
    val currentOnResult by rememberUpdatedState(onResult)
    val launcher = rememberLauncherForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        if (result.resultCode != Activity.RESULT_OK) return@rememberLauncherForActivityResult
        val contents = result.data?.getStringExtra(QrScannerActivity.EXTRA_RESULT)
        if (!contents.isNullOrBlank()) currentOnResult(contents)
    }
    return remember(launcher, prompt, context) {
        {
            launcher.launch(
                Intent(context, QrScannerActivity::class.java)
                    .putExtra(QrScannerActivity.EXTRA_PROMPT, prompt),
            )
        }
    }
}
