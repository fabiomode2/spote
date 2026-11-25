package dev.spote.app.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Home
import androidx.compose.material.icons.rounded.LibraryMusic
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.spote.app.model.Tab
import dev.spote.app.ui.theme.LocalSpoteColors

@Composable
fun SpoteBottomBar(selected: Tab, onSelect: (Tab) -> Unit) {
    val colors = LocalSpoteColors.current
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(colors.background)
            .navigationBarsPadding(),
    ) {
        Hairline()
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(58.dp)
                .padding(horizontal = 12.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            BottomItem(
                icon = Icons.Rounded.Home,
                label = "Inicio",
                selected = selected == Tab.Home,
                onClick = { onSelect(Tab.Home) },
                modifier = Modifier.weight(1f),
            )
            BottomItem(
                icon = Icons.Rounded.Search,
                label = "Buscar",
                selected = selected == Tab.Search,
                onClick = { onSelect(Tab.Search) },
                modifier = Modifier.weight(1f),
            )
            BottomItem(
                icon = Icons.Rounded.LibraryMusic,
                label = "Biblioteca",
                selected = selected == Tab.Library,
                onClick = { onSelect(Tab.Library) },
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun BottomItem(
    icon: ImageVector,
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LocalSpoteColors.current
    val tint = if (selected) colors.accent else MaterialTheme.colorScheme.onSurfaceVariant
    Column(
        modifier = modifier
            .clip(CircleShape)
            .clickable(onClick = onClick)
            .padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = label,
            tint = tint,
            modifier = Modifier.size(23.dp),
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall.copy(
                fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium,
            ),
            color = tint,
        )
    }
}
