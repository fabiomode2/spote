package dev.spote.app.ui.theme

enum class SpoteThemeMode(val label: String, val subtitle: String) {
    Midnight("Medianoche", "Carbón, lima y ritmo"),
    Nocturne("Nocturno", "Cálido, crema y serif"),
    ;

    fun colors(): SpoteColors = when (this) {
        Midnight -> SpoteColors.Night
        Nocturne -> SpoteColors.Nocturne
    }

    companion object {
        fun fromName(value: String?): SpoteThemeMode =
            entries.firstOrNull { it.name == value } ?: Midnight
    }
}
