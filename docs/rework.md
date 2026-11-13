# Revisión de la TUI (estado)

## Hecho

- **Layout declarativo** (`ui/layout.rs`): paneles apagables, docks
  izquierda/derecha, anchos/alto configurables, bloque centrado en vertical
  (`vcenter_pct`) y asas de arrastre. Editor en caliente con `E`; los paneles
  de lista ya no estiran una portada deformada.
- **Portada**: `default` (foto truecolor), `half` (media celda con paleta),
  `pixel` y `braille`; rotación
  apagada por defecto. Braille adaptativo: detecta papel claro/line-art,
  invierte la tinta, usa contraste local y conserva la proporción al cambiar
  de tamaño. En Kitty, `default` se envía como imagen PNG real a resolución
  completa; el half-block queda como fallback. Fondo `cover` fundido a pantalla
  completa (por defecto) y `lyrics` como textura, ambos mezclando solo el fondo de las
  celdas para no tapar el texto.
- **Efectos**: franja reducida a `clean` (barras suaves + marea),
  `aurora`, `bloom` e `inferno`; `void`/`neural`/`glitch` como fondo ambiental
  con opacidad ajustable. Bordes reactivos a bombo/caja/charles/voz, brasa
  continua en el borde inferior, halo de bloom en el play y derrame de `clean`.
  Todo en ajustes/`E`.
- **Audio**: mapa de energía de la canción completa (drops, tramos tranquilos)
  que sigue al seek y refina los tiempos de palabra de la letra. Curva de
  “feeling” y sección actual en el panel de amigos/transporte.
- **Letras**: matching reescrito (limpieza de títulos, variantes, duración,
  rechazo de versiones live/remix/karaoke y de syncs absurdas), parser LRC con
  offset y tags de palabra, refinado por energía vocal, resaltado palabra a
  palabra, tamaño de letra y clic en una frase para saltar. Las líneas que solo
  son adlibs entre paréntesis no roban la línea activa. `Home`/`End` recortan
  intro/outro de un vídeo y `shift+clic` sincroniza una línea con la posición
  actual.
- **Volumen**: slider con degradado, arrastre, rueda y flechas ↑↓ cuando el
  transporte tiene el foco o estás en «ahora». Los iconos del transporte
  escalan con el alto.
- **Búsqueda mixta**: artistas, álbumes y pistas en una sola vista, con
  portadas y sección por tipo; `z` alterna tarjetas/lista compacta. Las
  tarjetas de artista y álbum abren su ficha.
- **Álbumes**: sin pestaña aparte; página de álbum con portada, tracklist,
  descarga completa (`d`) y guardar como lista (`s`). Importación de álbum de
  Spotify.
- **Artistas**: perfil con imagen, géneros, fans, popularidad, top tracks y
  discografía; `enter` baja y reproduce, `a` encola, los álbumes abren su
  página. Los nombres de artista/álbum son clicables en casi toda la UI.
- **Biblioteca**: no se vacía si SQLite falla; filas con columnas fijas;
  `Delete`/clic derecho para acciones, borrar de la librería o del disco.
- **Cola**: arreglado el doble borrado del ratón; un clic en `×` quita una.
  Arriba se muestra siempre la pista que está sonando.
- **Sesión**: sustituida por amigos/presencia; la info técnica pasa a sala.
- **Importación**: solo Spotify (playlist o álbum). Deezer queda como
  respaldo de metadatos de búsqueda, no para listas.
- **Portadas de descargas**: se registra la miniatura de yt-dlp, pero se
  prefiere la carátula de Spotify; si falla la red se usa la local. La fila
  muestra el proveedor de la fuente (YouTube/SoundCloud/Spotify/local).
- **Menús contextuales**: cabecera de vistas y cola, pistas, resultados de
  artistas/álbumes y nombres clicables; fondo opaco y legible.
- **Discord**: Rich Presence con Application ID configurable, carátula,
  botón de Spotify y tiempo de reproducción.
- **Rendimiento**: 120 fps por defecto, búsquedas/cachés de SQLite para los
  contadores que se pintan cada frame y relleno de buffers en una pasada.

## Pendiente / limitaciones

- El “tamaño de letra” cambia ancho de columna y espaciado, no el tamaño de
  fuente real del terminal (eso no lo controla una TUI).
- La importación de playlists de Spotify vía embed devuelve como máximo 50
  pistas; para listas más largas hace falta token oficial.
- El perfil de artista usa la API/Deezer; si el token anónimo de Spotify está
  bloqueado, la discografía sale de Deezer.
- El mapa de audio decodifica la canción completa con ffmpeg en segundo plano;
  en pistas muy largas tarda unos segundos en aparecer.

## Ronda de pulido (overlays, letras y transporte)

- **Overlays**: fondo sólido + velo completo detrás; las celdas del modal se
  marcan `AlwaysUpdate` y al abrir/cerrar se fuerza un frame completo. Se
  acabaron los restos de la vista anterior a través de los bordes.
- **Letras**: las palabras entre paréntesis son opcionales para el progreso y
  la línea activa; una línea de adlib nunca roba el turno a la principal. El
  puente a lyricbar puede alinear la letra contra el audio local con
  faster-whisper (`aligned=true`) cuando el LRC viene estimado o de Genius.
  Las letras aproximadas cacheadas se reintentan una vez por sesión si hay
  audio local. `alt/ctrl/shift+clic` y el clic derecho sincronizan una línea.
- **Recorte**: chip `✂` en «ahora» y editor `T` con botones para marcar
  inicio/fin/quitar; marcas `┃` visibles en la barra de progreso.
- **Arte**: `half` es un modo real (media celda con paleta, sin Kitty) y entra
  en la rotación; `art_rotate_off` decide si `off` participa. `F` usa la
  portada full-res dentro del área sobre el transporte; `V` ya no pinta la
  portada detrás del visualizador (era el pixelado). El PNG se invalida al
  cambiar de tamaño.
- **Efectos**: pulsos más largos, brillo de bordes más continuo, luz de fondo
  ligada al bajo y `clean` recupera barras suaves + marea.
- **Transporte**: el playhead interpola entre eventos del reproductor y los
  cambios de layout fuerzan repintado completo para que botones/barra no se
  queden en posiciones viejas.
- **Descargas**: matching más estricto por título y artista; tolera intros de
  hasta 30 s con título exacto y prueba consultas de respaldo (`título`,
  `título artista`) antes de fallar.
- **Discord**: se quitó `large_text` del álbum para no duplicar la tercera
  línea de la actividad.
