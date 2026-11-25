package dev.spote.app.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil3.compose.AsyncImage
import dev.spote.app.model.Cover
import java.io.File

@Composable
fun CoverArt(
    cover: Cover,
    size: Dp,
    modifier: Modifier = Modifier,
    corner: Dp = 10.dp,
    labelSize: TextUnit = 22.sp,
) {
    val shape = RoundedCornerShape(corner)
    val imageModel: Any? = cover.localPath?.let(::File) ?: cover.remoteUrl
    Box(
        modifier = modifier
            .size(size)
            .clip(shape)
            .background(Brush.linearGradient(listOf(cover.start, cover.end)))
            .border(1.dp, Color.White.copy(alpha = 0.06f), shape),
        contentAlignment = Alignment.Center,
    ) {
        if (imageModel != null) {
            AsyncImage(
                model = imageModel,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
        } else {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(
                        Brush.verticalGradient(
                            listOf(
                                Color.White.copy(alpha = 0.14f),
                                Color.Transparent,
                                Color.Black.copy(alpha = 0.24f),
                            ),
                        ),
                    ),
            )
            if (size >= 56.dp) {
                Text(
                    text = cover.label,
                    color = Color.White.copy(alpha = 0.82f),
                    style = MaterialTheme.typography.headlineMedium.copy(
                        fontWeight = FontWeight.Bold,
                        fontSize = labelSize,
                        letterSpacing = (-0.5).sp,
                    ),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(4.dp),
                )
            }
        }
    }
}
