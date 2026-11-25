package dev.spote.app.setup

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/** Lado mínimo para que un QR siga siendo legible en pantalla. */
private const val MIN_QR_PX = 96

/**
 * Genera un QR como [Bitmap] sin abrir ninguna Activity.
 *
 * El tamaño solicitado es el lado del cuadrado; la matriz de ZXing ya incluye
 * el margen blanco (quiet zone) necesario para escanear.
 */
object QrCode {

    fun encode(text: String, sizePx: Int): Bitmap {
        require(text.isNotEmpty()) { "QR content must not be empty" }
        val dimension = sizePx.coerceAtLeast(MIN_QR_PX)
        val hints = mapOf<EncodeHintType, Any>(
            EncodeHintType.CHARACTER_SET to "UTF-8",
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M,
            EncodeHintType.MARGIN to 4,
        )
        val matrix = QRCodeWriter().encode(
            text,
            BarcodeFormat.QR_CODE,
            dimension,
            dimension,
            hints,
        )

        val pixels = IntArray(dimension * dimension)
        for (y in 0 until dimension) {
            for (x in 0 until dimension) {
                pixels[y * dimension + x] = if (matrix.get(x, y)) Color.BLACK else Color.WHITE
            }
        }
        return Bitmap.createBitmap(dimension, dimension, Bitmap.Config.ARGB_8888).apply {
            setPixels(pixels, 0, dimension, 0, 0, dimension, dimension)
        }
    }
}

/**
 * Dibuja [text] como QR. Si el contenido está vacío o ZXing falla, deja un
 * hueco del mismo tamaño en vez de tumbar la pantalla.
 */
@Composable
fun QrCodeImage(
    text: String,
    size: Dp,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val sizePx = with(density) { size.roundToPx() }.coerceAtLeast(MIN_QR_PX)
    val bitmap = remember(text, sizePx) {
        if (text.isBlank()) {
            null
        } else {
            runCatching { QrCode.encode(text, sizePx) }.getOrNull()
        }
    }

    if (bitmap != null) {
        Image(
            bitmap = bitmap.asImageBitmap(),
            contentDescription = "Código QR con el enlace de configuración",
            modifier = modifier.size(size),
            filterQuality = FilterQuality.None,
        )
    } else {
        Box(modifier = modifier.size(size))
    }
}
