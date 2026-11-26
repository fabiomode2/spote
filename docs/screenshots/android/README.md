# spote Android — capturas de la UI

Capturas reales de Compose renderizadas con Roborazzi (JVM, sin emulador) usando
datos mock. La carpeta hermana `android/` contiene el proyecto; las capturas se
regeneran con:

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 ./gradlew :app:recordRoborazziDebug \
  --tests 'dev.spote.app.screenshot.ScreenShotsTest'
```

| | |
|---|---|
| ![Home](01-home.png) | ![Buscar](02-search.png) |
| ![Buscar vacío](03-search-empty.png) | ![Biblioteca](04-library.png) |
| ![Álbum](05-album.png) | ![Now playing](06-now-playing.png) |
| ![Cola](08-queue.png) | ![Biblioteca grid](09-library-grid.png) |
| ![Playlist](10-playlist.png) | ![Descargas](11-downloads.png) |
| ![Ajustes](12-settings.png) | ![Sala](13-room.png) |
| ![Actualización](14-update-card.png) | ![Nocturno home](15-nocturne-home.png) |
| ![Nocturno now playing](16-nocturne-now-playing.png) | ![Nocturno álbum](17-nocturne-album.png) |
| ![Selector de tema](18-theme-picker.png) | ![Artista](19-artist.png) |
| ![Contact sheet](_contact-sheet.png) | |
