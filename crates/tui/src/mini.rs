//! Servidor del mini reproductor: publica el estado por un socket Unix y
//! recibe comandos. El cliente vive en `crates/mini` (`spote-mini`).

use std::io::{BufReader, ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use spote_protocol::mini::{mini_socket_path, MiniCommand, MiniSnapshot};

const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);

pub struct MiniHub {
    clients: Arc<Mutex<Vec<UnixStream>>>,
    commands: Receiver<MiniCommand>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    worker: Option<JoinHandle<()>>,
}

impl MiniHub {
    pub fn start() -> std::io::Result<Self> {
        let path = mini_socket_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Un socket huérfano de una ejecución anterior se puede borrar; uno
        // vivo significa que ya hay otra instancia de spote sirviendo el mini.
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    ErrorKind::AddrInUse,
                    "ya hay otra instancia de spote con el socket mini activo",
                ));
            }
            let _ = std::fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        // El socket solo debe ser accesible por el usuario que abrió spote.
        // Especialmente importante en el fallback a /tmp.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        listener.set_nonblocking(true)?;

        let clients: Arc<Mutex<Vec<UnixStream>>> = Arc::new(Mutex::new(Vec::new()));
        let (command_tx, command_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_clients = Arc::clone(&clients);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("spote-mini-hub".to_string())
            .spawn(move || serve(listener, worker_clients, command_tx, worker_stop))?;

        Ok(Self {
            clients,
            commands: command_rx,
            stop,
            path,
            worker: Some(worker),
        })
    }

    pub fn publish(&self, snapshot: &MiniSnapshot) {
        let Ok(line) = snapshot.to_line() else {
            return;
        };
        let mut clients = match self.clients.lock() {
            Ok(clients) => clients,
            Err(_) => return,
        };
        // Si un cliente no lee, se cierra: dejar una línea a medias y
        // reutilizar el socket rompería el framing NDJSON.
        clients.retain_mut(|stream| stream.write_all(line.as_bytes()).is_ok());
    }

    pub fn has_clients(&self) -> bool {
        self.clients
            .lock()
            .map(|clients| !clients.is_empty())
            .unwrap_or(false)
    }

    pub fn try_recv(&self) -> Option<MiniCommand> {
        self.commands.try_recv().ok()
    }
}

impl Drop for MiniHub {
    fn drop(&mut self) {
        if let Ok(mut clients) = self.clients.lock() {
            for stream in clients.iter_mut() {
                let _ = stream.write_all(b"\"Quit\"\n");
            }
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(
    listener: UnixListener,
    clients: Arc<Mutex<Vec<UnixStream>>>,
    commands: Sender<MiniCommand>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                match stream.try_clone() {
                    Ok(writer) => {
                        // O_NONBLOCK se comparte entre clones: se usan
                        // timeouts de lectura/escritura, no modo no bloqueante.
                        let _ = writer.set_write_timeout(Some(Duration::from_millis(5)));
                        if let Ok(mut list) = clients.lock() {
                            list.push(writer);
                        }
                        let reader_commands = commands.clone();
                        let _ = thread::Builder::new()
                            .name("spote-mini-client".to_string())
                            .spawn(move || read_client(stream, reader_commands));
                    }
                    Err(_) => {}
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                thread::sleep(PUBLISH_INTERVAL);
            }
            Err(_) => thread::sleep(PUBLISH_INTERVAL),
        }
    }
}

fn read_client(stream: UnixStream, commands: Sender<MiniCommand>) {
    const MAX_LINE: usize = 64 * 1024;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut reader = BufReader::new(stream);
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                for &byte in &chunk[..n] {
                    if byte == b'\n' {
                        let text = String::from_utf8_lossy(&pending).into_owned();
                        pending.clear();
                        let Ok(command) = MiniCommand::from_line(text.trim()) else {
                            continue;
                        };
                        if command.is_quit() || commands.send(command).is_err() {
                            return;
                        }
                    } else {
                        pending.push(byte);
                        if pending.len() > MAX_LINE {
                            // Cliente local que no termina la línea: se corta
                            // para no agotar memoria en el hilo de la TUI.
                            return;
                        }
                    }
                }
            }
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue;
            }
            Err(_) => break,
        }
    }
}
