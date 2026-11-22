//! Worker del reproductor MPRIS del sistema.
//!
//! El watcher de `spote_core` es síncrono y bloquea mientras habla con D-Bus,
//! así que vive en su propio hilo. La UI solo lee estados y encola comandos.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use spote_core::system_media::{SystemMediaState, SystemMediaWatcher};

#[derive(Debug, Clone, PartialEq)]
pub enum SystemCommand {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    Stop,
    SetPosition(u64),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeat(String),
}

pub struct SystemMediaWorker {
    states: Receiver<SystemMediaState>,
    commands: Sender<SystemCommand>,
    active: Arc<AtomicBool>,
}

impl SystemMediaWorker {
    pub fn start() -> Self {
        let (state_tx, state_rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let active = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&active);
        thread::Builder::new()
            .name("spote-system-media".into())
            .spawn(move || run(state_tx, command_rx, flag))
            .ok();
        Self {
            states: state_rx,
            commands: command_tx,
            active,
        }
    }

    pub fn try_recv(&self) -> Option<SystemMediaState> {
        self.states.try_recv().ok()
    }

    pub fn send(&self, command: SystemCommand) {
        let _ = self.commands.send(command);
    }
}

impl Drop for SystemMediaWorker {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
    }
}

fn run(
    states: Sender<SystemMediaState>,
    commands: Receiver<SystemCommand>,
    active: Arc<AtomicBool>,
) {
    let mut watcher = SystemMediaWatcher::new();
    let mut last_key = String::new();
    let mut sent_empty = false;
    while active.load(Ordering::Relaxed) {
        // Primero los comandos: el usuario espera respuesta inmediata.
        while let Ok(command) = commands.try_recv() {
            match command {
                SystemCommand::Play => {
                    let _ = watcher.call("Play");
                }
                SystemCommand::Pause => {
                    let _ = watcher.call("Pause");
                }
                SystemCommand::PlayPause => {
                    let _ = watcher.call("PlayPause");
                }
                SystemCommand::Next => {
                    let _ = watcher.call("Next");
                }
                SystemCommand::Previous => {
                    let _ = watcher.call("Previous");
                }
                SystemCommand::Stop => {
                    let _ = watcher.call("Stop");
                }
                SystemCommand::SetPosition(position) => {
                    if !last_key.is_empty() {
                        let _ = watcher.set_position(&last_key, position);
                    }
                }
                SystemCommand::SetVolume(volume) => {
                    let _ = watcher.set_volume(volume);
                }
                SystemCommand::SetShuffle(on) => {
                    let _ = watcher.set_shuffle(on);
                }
                SystemCommand::SetRepeat(mode) => {
                    let _ = watcher.set_repeat(&mode);
                }
            }
        }
        let state = watcher.poll().unwrap_or_default();
        let empty = state.track.is_none();
        if let Some(track) = &state.track {
            last_key = track.key.clone();
            sent_empty = false;
        }
        // Un estado vacío también es información: si el reproductor cierra,
        // la UI/mini/MPRIS no deben seguir mostrando la última canción.
        if !empty || !sent_empty {
            sent_empty = empty;
            if states.send(state).is_err() {
                return;
            }
        }
        thread::sleep(Duration::from_millis(400));
    }
}
