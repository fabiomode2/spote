//! Acciones y formularios de la parte social que vive en la TUI.
//!
//! La red la lleva `social.rs`; aquí solo se preparan las filas, se abren los
//! modales y se traduce la elección del usuario a `SocialCmd`.

use super::*;

/// Una persona elegible en el selector social.
#[derive(Debug, Clone)]
pub struct SocialPickRow {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub friend: bool,
    pub detail: String,
}

impl App {
    /// `true` si la sala actual es nuestra (lo sabe el servidor por el login).
    pub fn is_room_host(&self) -> bool {
        self.room
            .as_ref()
            .is_some_and(|room| !room.host_id.is_empty() && room.host_id == self.social_user_id)
    }

    pub fn is_friend(&self, user_id: &str) -> bool {
        self.friends.iter().any(|user| user.id == user_id)
    }

    pub fn open_create_room(&mut self) {
        if self.social.is_none() {
            self.toast("sin servidor social · configura [server]", ToastKind::Warn);
            return;
        }
        self.room_form = RoomForm::default();
        self.overlay = Overlay::CreateRoom;
    }

    pub fn commit_room_form(&mut self) {
        let name = self.room_form.name.trim().to_string();
        if name.is_empty() {
            self.toast("la sala necesita un nombre", ToastKind::Warn);
            return;
        }
        let is_public = self.room_form.is_public;
        if let Some(social) = &self.social {
            social.send(SocialCmd::CreateRoom { name, is_public });
            self.toast("creando sala…", ToastKind::Info);
        }
        self.overlay = Overlay::None;
    }

    pub fn open_social_people(&mut self, mode: SocialPeopleMode) {
        if self.social.is_none() {
            self.toast("sin servidor social · configura [server]", ToastKind::Warn);
            return;
        }
        if matches!(mode, SocialPeopleMode::Invite | SocialPeopleMode::Kick) && self.room.is_none()
        {
            self.toast("entra en una sala primero", ToastKind::Warn);
            return;
        }
        if matches!(mode, SocialPeopleMode::Invite | SocialPeopleMode::Kick) && !self.is_room_host()
        {
            self.toast("solo el host puede gestionar la sala", ToastKind::Warn);
            return;
        }
        self.overlay = Overlay::SocialPeople { mode, selected: 0 };
    }

    /// Filas del selector según el modo. Se recalculan al abrir, mover y
    /// confirmar: la presencia cambia mientras el modal está abierto.
    pub fn social_pick_rows(&self, mode: SocialPeopleMode) -> Vec<SocialPickRow> {
        let me = self.social_user_id.as_str();
        let mut rows: Vec<SocialPickRow> = match mode {
            SocialPeopleMode::Friend => self
                .presence
                .iter()
                .filter(|presence| presence.user.id != me)
                .map(|presence| SocialPickRow {
                    id: presence.user.id.clone(),
                    name: presence.user.name.clone(),
                    online: presence.online,
                    friend: self.is_friend(&presence.user.id),
                    detail: presence
                        .now_playing
                        .as_ref()
                        .map(|track| track.label())
                        .unwrap_or_else(|| {
                            if presence.online {
                                "en línea".into()
                            } else {
                                "desconectado".into()
                            }
                        }),
                })
                .collect(),
            SocialPeopleMode::Invite => {
                let members: Vec<&str> = self
                    .room
                    .as_ref()
                    .map(|room| {
                        room.members
                            .iter()
                            .map(|member| member.user.id.as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                self.presence
                    .iter()
                    .filter(|presence| presence.user.id != me)
                    .filter(|presence| !members.contains(&presence.user.id.as_str()))
                    .map(|presence| SocialPickRow {
                        id: presence.user.id.clone(),
                        name: presence.user.name.clone(),
                        online: presence.online,
                        friend: self.is_friend(&presence.user.id),
                        detail: presence
                            .now_playing
                            .as_ref()
                            .map(|track| track.label())
                            .unwrap_or_else(|| {
                                if presence.online {
                                    "en línea".into()
                                } else {
                                    "desconectado".into()
                                }
                            }),
                    })
                    .collect()
            }
            SocialPeopleMode::Kick => {
                let Some(room) = self.room.as_ref() else {
                    return Vec::new();
                };
                room.members
                    .iter()
                    .filter(|member| member.user.id != me)
                    .filter(|member| member.role != spote_protocol::Role::Host)
                    .map(|member| {
                        let presence = self
                            .presence
                            .iter()
                            .find(|presence| presence.user.id == member.user.id);
                        SocialPickRow {
                            id: member.user.id.clone(),
                            name: member.user.name.clone(),
                            online: presence.is_some_and(|presence| presence.online),
                            friend: self.is_friend(&member.user.id),
                            detail: presence
                                .and_then(|presence| presence.now_playing.as_ref())
                                .map(|track| track.label())
                                .unwrap_or_else(|| "sin reproducción".into()),
                        }
                    })
                    .collect()
            }
        };
        // Amistad: primero quien aún no es amigo y está online. Invitar:
        // primero amigos y online. Expulsar: por nombre.
        match mode {
            SocialPeopleMode::Friend => {
                rows.sort_by_key(|row| (row.friend, !row.online, row.name.to_lowercase()));
            }
            SocialPeopleMode::Invite => {
                rows.sort_by_key(|row| (!row.friend, !row.online, row.name.to_lowercase()));
            }
            SocialPeopleMode::Kick => {
                rows.sort_by_key(|row| row.name.to_lowercase());
            }
        }
        rows
    }

    pub fn commit_social_pick(&mut self) {
        let Overlay::SocialPeople { mode, selected } = self.overlay.clone() else {
            return;
        };
        let rows = self.social_pick_rows(mode);
        let Some(row) = rows.get(selected).cloned() else {
            return;
        };
        match mode {
            SocialPeopleMode::Friend => {
                if row.friend {
                    self.toast(format!("{} ya es amigo", row.name), ToastKind::Info);
                    self.overlay = Overlay::None;
                    return;
                }
                if let Some(social) = &self.social {
                    social.send(SocialCmd::AddFriend(row.id));
                    self.toast(format!("añadiendo a {}…", row.name), ToastKind::Info);
                }
            }
            SocialPeopleMode::Invite => {
                let Some(room) = self.room.as_ref().map(|room| room.id.clone()) else {
                    self.overlay = Overlay::None;
                    return;
                };
                if let Some(social) = &self.social {
                    social.send(SocialCmd::Invite { room, user: row.id });
                    self.toast(format!("invitando a {}…", row.name), ToastKind::Info);
                }
            }
            SocialPeopleMode::Kick => {
                let Some(room) = self.room.as_ref().map(|room| room.id.clone()) else {
                    self.overlay = Overlay::None;
                    return;
                };
                if let Some(social) = &self.social {
                    social.send(SocialCmd::Kick { room, user: row.id });
                    self.toast(format!("expulsando a {}…", row.name), ToastKind::Info);
                }
            }
        }
        self.overlay = Overlay::None;
    }

    pub fn open_chat(&mut self) {
        if self.social.is_none() {
            self.toast("sin servidor social · configura [server]", ToastKind::Warn);
            return;
        }
        if self.room.is_none() {
            self.toast("entra en una sala para hablar", ToastKind::Warn);
            return;
        }
        self.chat_input.clear();
        self.overlay = Overlay::Chat;
    }

    pub fn send_chat(&mut self) {
        let text = self.chat_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.room.is_none() {
            self.overlay = Overlay::None;
            return;
        }
        if let Some(social) = &self.social {
            social.send(SocialCmd::Chat(text));
        }
        self.chat_input.clear();
    }

    /// Teclado propio de la pestaña de salas.
    pub fn handle_room_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.intersects(
            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
        ) {
            return false;
        }
        match key.code {
            KeyCode::Char('n') => self.open_create_room(),
            KeyCode::Char('a') => self.open_social_people(SocialPeopleMode::Friend),
            KeyCode::Char('i') => self.open_social_people(SocialPeopleMode::Invite),
            KeyCode::Char('x') => self.open_social_people(SocialPeopleMode::Kick),
            KeyCode::Char('t') => self.open_chat(),
            _ => return false,
        }
        true
    }

    /// Teclado de los modales sociales. Devuelve si el overlay era social
    /// (para no caer en el manejo general de overlays).
    pub fn handle_social_overlay_key(&mut self, key: KeyEvent) -> bool {
        match self.overlay.clone() {
            Overlay::CreateRoom => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Tab | KeyCode::BackTab => {
                    self.room_form.is_public = !self.room_form.is_public;
                }
                KeyCode::Enter => self.commit_room_form(),
                KeyCode::Backspace => {
                    self.room_form.name.pop();
                }
                KeyCode::Char(c) if !c.is_control() => self.room_form.name.push(c),
                _ => {}
            },
            Overlay::SocialPeople { mode, selected } => {
                let len = self.social_pick_rows(mode).len();
                match key.code {
                    KeyCode::Esc => self.overlay = Overlay::None,
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.overlay = Overlay::SocialPeople {
                            mode,
                            selected: selected.saturating_sub(1),
                        };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.overlay = Overlay::SocialPeople {
                            mode,
                            selected: (selected + 1).min(len.saturating_sub(1)),
                        };
                    }
                    KeyCode::Home => {
                        self.overlay = Overlay::SocialPeople { mode, selected: 0 };
                    }
                    KeyCode::End => {
                        self.overlay = Overlay::SocialPeople {
                            mode,
                            selected: len.saturating_sub(1),
                        };
                    }
                    KeyCode::Enter => self.commit_social_pick(),
                    _ => {}
                }
            }
            Overlay::Chat => match key.code {
                KeyCode::Esc => self.overlay = Overlay::None,
                KeyCode::Enter => self.send_chat(),
                KeyCode::Backspace => {
                    self.chat_input.pop();
                }
                KeyCode::Char(c) if !c.is_control() => self.chat_input.push(c),
                _ => {}
            },
            _ => return false,
        }
        true
    }
}
