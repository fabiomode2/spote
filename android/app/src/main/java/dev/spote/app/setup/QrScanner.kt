package dev.spote.app.setup

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import com.google.zxing.BarcodeFormat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions

private const val DEFAULT_SCAN_PROMPT = "Apunta al QR de configuración de Spote"

/** Opciones del escáner embebido; la UI puede personalizar el texto. */
fun qrScanOptions(prompt: String = DEFAULT_SCAN_PROMPT): ScanOptions =
    ScanOptions().apply {
        setDesiredBarcodeFormats(BarcodeFormat.QR_CODE.name)
        setPrompt(prompt)
        setBeepEnabled(false)
        setOrientationLocked(false)
    }

/**
 * Devuelve una función que abre el escáner de ZXing y entrega el texto leído.
 *
 * Uso desde un composable:
 *
 * ```
 * val scan = rememberQrScanner { raw -> SetupLink.parse(raw)?.let { ... } }
 * Button(onClick = scan) { Text("Escanear QR") }
 * ```
 *
 * Si el usuario cancela no se llama a [onResult]. La Activity de captura la
 * aporta `zxing-android-embedded`; no hace falta declarar ninguna propia.
 */
@Composable
fun rememberQrScanner(
    onResult: (String) -> Unit,
    prompt: String = DEFAULT_SCAN_PROMPT,
): () -> Unit {
    val currentOnResult by rememberUpdatedState(onResult)
    val launcher = rememberLauncherForActivityResult(ScanContract()) { result ->
        val contents = result.contents
        if (!contents.isNullOrBlank()) currentOnResult(contents)
    }
    return remember(launcher, prompt) {
        { launcher.launch(qrScanOptions(prompt)) }
    }
}
