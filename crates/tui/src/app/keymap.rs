//! Fuente única de leyendas y ayuda. Si una tecla no está aquí, no existe:
//! así la barra inferior no miente.

use super::{App, CenterFocus, Overlay, Pane, Tab};

#[derive(Debug, Clone, Copy)]
pub struct Hint {
    pub key: &'static str,
    pub label: &'static str,
}

pub fn help_sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            "reproducción",
            vec![
                ("space", "play / pausa"),
                ("n / p", "siguiente / anterior"),
                ("S", "aleatorio on/off"),
                ("o", "bucle: off / todo / una"),
                ("u", "bucle de la pista"),
                ("← →", "seek ±5 s (shift ±30 s)"),
                ("↑ ↓", "volumen en ahora; selección en listas"),
                ("J / K", "mover selección siempre"),
                (", .", "offset de letra si estás en la letra"),
                ("m", "silenciar"),
            ],
        ),
        (
            "ahora / letra",
            vec![
                ("c", "portada a todo el centro"),
                ("C / l", "letra a todo el centro"),
                ("clic en frase", "salta a ese momento"),
                ("rueda / ↑↓", "scroll manual de la letra"),
                ("A− A+", "tamaño de la letra (clic)"),
                ("±250 / ±100", "offset de letra (clic)"),
                ("⟲", "volver a la línea actual"),
                ("⟳", "buscar otra letra y borrar la caché"),
                ("b / ⌕", "búsqueda manual con candidatos y vista previa"),
                ("M", "siguiente modo de arte"),
                ("r", "rotación automática del arte"),
                ("F", "portada a pantalla completa"),
                ("i", "zen: solo efectos"),
                ("T / clic ✂", "abrir editor de recorte"),
                ("home / end", "recortar intro / outro aquí"),
                ("backspace", "quitar recorte"),
                ("shift/alt/clic-der", "sincronizar la letra con este punto"),
            ],
        ),
        (
            "biblioteca / búsqueda",
            vec![
                ("/", "buscar en la biblioteca (inline)"),
                ("enter", "reproducir / buscar"),
                ("↑ ↓", "mover selección"),
                ("a", "añadir a la cola"),
                ("f", "favorita"),
                ("g", "perfil de artista"),
                ("clic derecho", "acciones / borrar / estado"),
                ("R", "recargar biblioteca"),
                ("X / ⇧clic ×", "quitar de la biblioteca"),
            ],
        ),
        (
            "interfaz",
            vec![
                ("E", "editor de diseño (tamaños y paneles)"),
                ("e", "editor de capas del visualizador"),
                ("v", "siguiente visualización"),
                ("V", "visualización a pantalla completa"),
                ("b", "visualizador de fondo"),
                ("z", "lista compacta / tarjetas"),
                ("t", "seguir el sistema (MPRIS)"),
                ("w", "mini reproductor de escritorio"),
                ("W", "resumen de escucha (Wrapped)"),
                ("A", "pestaña de audio: EQ y nivelación"),
                ("tab", "cambiar de vista"),
                ("q", "salir de spote"),
            ],
        ),
        (
            "listas / salas",
            vec![
                ("N", "crear lista"),
                ("I", "importar lista o álbum de Spotify"),
                ("l", "añadir a lista (en biblioteca)"),
                ("d", "borrar la lista abierta"),
                ("[ / ]", "ciclar filtros y listas"),
                ("enter", "unirse a sala"),
                ("L", "salir de la sala"),
                ("s", "refrescar salas"),
            ],
        ),
        (
            "sistema",
            vec![
                ("x", "quitar de la cola"),
                ("y", "copiar letra"),
                ("F2", "harvest"),
                ("?", "esta ayuda"),
                ("q / ctrl-c", "salir"),
            ],
        ),
    ]
}

impl App {
    pub fn hints(&self) -> Vec<Hint> {
        if self.overlay != Overlay::None {
            return match self.overlay {
                Overlay::Context => vec![
                    Hint { key: "↑↓", label: "elegir" },
                    Hint { key: "enter", label: "aplicar" },
                    Hint { key: "supr", label: "borrar" },
                    Hint { key: "esc", label: "cerrar" },
                ],
                Overlay::Layout => vec![
                    Hint { key: "↑↓", label: "elegir" },
                    Hint { key: "←→", label: "tamaño" },
                    Hint { key: "enter", label: "on/off" },
                    Hint { key: "r", label: "reset" },
                    Hint { key: "E", label: "cerrar" },
                ],
                Overlay::Artist => vec![
                    Hint { key: "↑↓", label: "elegir" },
                    Hint { key: "enter", label: "bajar y sonar" },
                    Hint { key: "a", label: "encolar" },
                    Hint { key: "esc", label: "cerrar" },
                ],
                Overlay::Effects => vec![
                    Hint { key: "1-9", label: "capas" },
                    Hint { key: "e", label: "cerrar" },
                ],
                Overlay::Update => vec![
                    Hint { key: "enter", label: "actualizar" },
                    Hint { key: "esc", label: "luego" },
                ],
                Overlay::Trim => vec![
                    Hint { key: "a", label: "marcar inicio" },
                    Hint { key: "b", label: "marcar fin" },
                    Hint { key: "c", label: "quitar" },
                    Hint { key: "esc", label: "cerrar" },
                ],
                _ => vec![Hint { key: "esc", label: "cerrar" }],
            };
        }
        if let Some(detail) = self.detail {
            return match detail {
                crate::app::Detail::Artist => vec![
                    Hint { key: "↑↓", label: "elegir" },
                    Hint { key: "enter", label: "sonar" },
                    Hint { key: "a", label: "cola" },
                    Hint { key: "[ ]", label: "discografía" },
                    Hint { key: "esc", label: "volver" },
                    Hint { key: "?", label: "ayuda" },
                ],
                crate::app::Detail::Album => vec![
                    Hint { key: "↑↓", label: "elegir" },
                    Hint { key: "enter", label: "sonar" },
                    Hint { key: "a", label: "cola" },
                    Hint { key: "d", label: "bajar" },
                    Hint { key: "s", label: "lista" },
                    Hint { key: "esc", label: "volver" },
                ],
            };
        }
        if self.center_focus == CenterFocus::Lyrics && self.tab == Tab::Now {
            if self.lyrics_search.is_some() {
                return vec![
                    Hint { key: "escribe", label: "consulta" },
                    Hint { key: "enter", label: "buscar / aceptar" },
                    Hint { key: "↑↓", label: "elegir versión" },
                    Hint { key: "PgUp/Dn", label: "preview" },
                    Hint { key: "esc", label: "volver" },
                ];
            }
            return vec![
                Hint { key: "espacio", label: "play" },
                Hint { key: ", .", label: "offset ±100 ms" },
                Hint { key: "↑↓", label: "scroll letra" },
                Hint { key: "+ −", label: "zoom letra" },
                Hint { key: "b", label: "buscar letra a mano" },
                Hint { key: "rueda", label: "scroll letra" },
                Hint { key: "clic", label: "salta a la frase" },
                Hint { key: "alt/der", label: "sincronizar letra" },
                Hint { key: "D", label: "debug letra" },
                Hint { key: "esc", label: "volver" },
            ];
        }
        match self.tab {
            Tab::Now => {
                let mut hints = vec![
                    Hint { key: "espacio", label: "play" },
                    Hint { key: "←→", label: "seek" },
                    Hint { key: "↑↓", label: "volumen" },
                    Hint { key: "a", label: "cola" },
                    Hint { key: "u", label: "bucle" },
                    Hint { key: "c", label: "portada" },
                    Hint { key: "C", label: "letra" },
                    Hint { key: "D", label: "debug letra" },
                    Hint { key: "M", label: "arte" },
                    Hint { key: "T", label: "recorte" },
                    Hint { key: "E", label: "diseño" },
                ];
                if self.focus == Pane::Transport {
                    hints[2] = Hint { key: "↑↓", label: "volumen · foco" };
                }
                hints.push(Hint { key: "?", label: "ayuda" });
                hints
            }
            Tab::Library => vec![
                Hint { key: "espacio", label: "play" },
                Hint { key: "↑↓", label: "mover" },
                Hint { key: "a", label: "cola" },
                Hint { key: "f", label: "favorita" },
                Hint { key: "g", label: "artista" },
                Hint { key: "clic-der", label: "acciones" },
                Hint { key: "X", label: "quitar" },
                Hint { key: "supr", label: "menú" },
                Hint { key: "M", label: "arte" },
                Hint { key: "E", label: "diseño" },
                Hint { key: "?", label: "ayuda" },
            ],
            Tab::Albums => {
                if self.album_open.is_some() {
                    vec![
                        Hint { key: "enter", label: "reproducir" },
                        Hint { key: "↑↓", label: "mover" },
                        Hint { key: "d", label: "bajar álbum" },
                        Hint { key: "s", label: "guardar lista" },
                        Hint { key: "esc", label: "volver" },
                        Hint { key: "?", label: "ayuda" },
                    ]
                } else {
                    vec![
                        Hint { key: "enter", label: "abrir álbum" },
                        Hint { key: "↑↓", label: "mover" },
                        Hint { key: "d", label: "bajar todo" },
                        Hint { key: "s", label: "guardar lista" },
                        Hint { key: "g", label: "artista" },
                        Hint { key: "E", label: "diseño" },
                        Hint { key: "?", label: "ayuda" },
                    ]
                }
            }
            Tab::Search => vec![
                Hint { key: "escribe", label: "buscar" },
                Hint { key: "enter", label: "buscar / bajar" },
                Hint { key: "↑↓", label: "mover" },
                Hint { key: "a", label: "cola" },
                Hint { key: "clic-der", label: "acciones" },
                Hint { key: "esc", label: "limpiar" },
                Hint { key: "?", label: "ayuda" },
            ],
            Tab::Audio => vec![
                Hint { key: "←→", label: "cambiar control" },
                Hint { key: "↑↓", label: "ajustar valor" },
                Hint { key: "enter", label: "alternar" },
                Hint { key: "p", label: "preset" },
                Hint { key: "n", label: "nivelar on/off" },
                Hint { key: "esc", label: "volver a ahora" },
                Hint { key: "?", label: "ayuda" },
            ],
            Tab::Room => vec![
                Hint { key: "enter", label: "entrar" },
                Hint { key: "L", label: "salir" },
                Hint { key: "s", label: "refrescar" },
                Hint { key: "g", label: "artista" },
                Hint { key: "E", label: "diseño" },
                Hint { key: "?", label: "ayuda" },
            ],
            Tab::Settings => vec![
                Hint { key: "↑↓", label: "mover" },
                Hint { key: "←→", label: "cambiar" },
                Hint { key: "enter", label: "alternar" },
                Hint { key: "E", label: "diseño" },
                Hint { key: "?", label: "ayuda" },
            ],
        }
    }
}
