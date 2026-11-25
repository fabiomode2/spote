# Reglas para cuando activemos R8. De momento release compila sin minificar.
-keep class dev.spote.app.update.** { *; }
-keep class dev.spote.rust.** { *; }
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
