//! Cliente del socket Unix del mini reproductor.
//!
//! Dos hilos mantienen la conexión fuera de la UI:
//! - el lector conecta, lee snapshots, guarda el último y avisa de los cambios
//!   de conexión;
//! - el escritor recibe comandos y los manda por un clon del mismo socket.
//!
//! La UI solo hace `try_recv`, `take_snapshot` y `send`; nunca toca el socket.

use spote_protocol::mini::{MiniCommand, MiniSnapshot};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(8);
const READ_CHUNK: usize = 8 * 1024;
const MAX_LINE_BUFFER: usize = 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

/// Cambios de conexión que la UI necesita ver. El último snapshot va aparte,
/// en [`MiniLink::take_snapshot`], para no acumular estados viejos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetEvent {
    Connected,
    Disconnected,
    /// El servidor pidió cerrar el mini reproductor.
    Quit,
}

enum WriterMsg {
    /// Clon del socket recién conectado, o `None` al desconectar.
    Stream(Option<UnixStream>),
    Command(MiniCommand),
}

/// Enlace con el proceso principal. Al soltarse manda `Quit` y espera a los dos
/// hilos.
pub struct MiniLink {
    events: Receiver<NetEvent>,
    commands: Sender<WriterMsg>,
    latest: Arc<Mutex<Option<MiniSnapshot>>>,
    shutdown: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    pub socket_path: PathBuf,
}

impl MiniLink {
    pub fn spawn(socket_path: PathBuf) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (writer_tx, writer_rx) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));

        let writer = thread::Builder::new()
            .name("spote-mini-writer".to_owned())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                move || writer_loop(writer_rx, shutdown)
            })
            .ok();

        let reader = thread::Builder::new()
            .name("spote-mini-reader".to_owned())
            .spawn({
                let path = socket_path.clone();
                let latest = Arc::clone(&latest);
                let shutdown = Arc::clone(&shutdown);
                let writer = writer_tx.clone();
                move || reader_loop(path, event_tx, latest, writer, shutdown)
            })
            .ok();

        Self {
            events: event_rx,
            commands: writer_tx,
            latest,
            shutdown,
            reader,
            writer,
            socket_path,
        }
    }

    /// Evento pendiente, sin bloquear. La UI lo llama en cada frame.
    pub fn try_recv(&self) -> Option<NetEvent> {
        self.events.try_recv().ok()
    }

    /// Último snapshot recibido, si cambió desde la última llamada.
    pub fn take_snapshot(&self) -> Option<MiniSnapshot> {
        self.latest.lock().ok().and_then(|mut slot| slot.take())
    }

    /// Encola un comando. Si no hay conexión, el escritor lo descarta.
    pub fn send(&self, command: MiniCommand) {
        let _ = self.commands.send(WriterMsg::Command(command));
    }

    /// Cierra el socket y espera a los hilos (máximo unos cientos de ms).
    pub fn shutdown(&mut self) {
        // Primero el comando, para que el escritor lo reciba con el socket vivo;
        // después la bandera que hace salir al lector.
        let _ = self.commands.send(WriterMsg::Command(MiniCommand::Quit));
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for MiniLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn reader_loop(
    path: PathBuf,
    events: Sender<NetEvent>,
    latest: Arc<Mutex<Option<MiniSnapshot>>>,
    writer: Sender<WriterMsg>,
    shutdown: Arc<AtomicBool>,
) {
    let mut stream: Option<UnixStream> = None;
    let mut connected = false;
    let mut last_attempt: Option<Instant> = None;
    let mut read_buf = Vec::with_capacity(READ_CHUNK);
    let mut chunk = [0u8; READ_CHUNK];

    while !shutdown.load(Ordering::Relaxed) {
        // Reconexión perezosa: un intento por segundo, sin bloquear la UI.
        if stream.is_none()
            && last_attempt
                .map(|at| at.elapsed() >= RETRY_INTERVAL)
                .unwrap_or(true)
        {
            last_attempt = Some(Instant::now());
            if let Ok(socket) = UnixStream::connect(&path) {
                if socket.set_nonblocking(true).is_ok() {
                    match socket.try_clone() {
                        Ok(write_socket) => {
                            let _ = writer.send(WriterMsg::Stream(Some(write_socket)));
                            stream = Some(socket);
                            connected = true;
                            read_buf.clear();
                            if events.send(NetEvent::Connected).is_err() {
                                let _ = writer.send(WriterMsg::Stream(None));
                                return;
                            }
                        }
                        Err(_) => {
                            // Sin clon no hay escritor; se reintenta la conexión.
                        }
                    }
                }
            }
        }

        let mut drop_connection = false;

        if let Some(socket) = stream.as_mut() {
            // Lectura no bloqueante hasta vaciar el socket.
            loop {
                match socket.read(&mut chunk) {
                    Ok(0) => {
                        drop_connection = true;
                        break;
                    }
                    Ok(n) => {
                        read_buf.extend_from_slice(&chunk[..n]);
                        if read_buf.len() > MAX_LINE_BUFFER {
                            // Línea corrupta o gigante: se tira y se sigue.
                            read_buf.clear();
                        }
                        if !drain_lines(&mut read_buf, &events, &latest) {
                            let _ = writer.send(WriterMsg::Stream(None));
                            return;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        drop_connection = true;
                        break;
                    }
                }
            }
        }

        if drop_connection {
            stream = None;
            read_buf.clear();
            last_attempt = Some(Instant::now());
            if let Ok(mut slot) = latest.lock() {
                *slot = None;
            }
            let _ = writer.send(WriterMsg::Stream(None));
            if connected {
                connected = false;
                if events.send(NetEvent::Disconnected).is_err() {
                    return;
                }
            }
        }

        thread::sleep(if stream.is_some() {
            TICK
        } else {
            Duration::from_millis(25)
        });
    }

    let _ = writer.send(WriterMsg::Stream(None));
}

fn writer_loop(messages: Receiver<WriterMsg>, shutdown: Arc<AtomicBool>) {
    let mut stream: Option<UnixStream> = None;

    loop {
        let message = match messages.recv_timeout(Duration::from_millis(250)) {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };

        match message {
            WriterMsg::Stream(new_stream) => {
                stream = new_stream;
            }
            WriterMsg::Command(command) => {
                let quit = command.is_quit();
                if let Ok(line) = command.to_line() {
                    let failed = match stream.as_mut() {
                        Some(socket) => write_nonblocking(socket, line.as_bytes()).is_err(),
                        None => false,
                    };
                    if failed {
                        stream = None;
                    }
                }
                if quit {
                    break;
                }
            }
        }
    }
}

/// Escritura corta con reintento: el socket es no bloqueante (lo comparte con
/// el lector), así que un `WouldBlock` se espera unos milisegundos.
fn write_nonblocking(socket: &mut UnixStream, mut data: &[u8]) -> std::io::Result<()> {
    let deadline = Instant::now() + WRITE_TIMEOUT;
    while !data.is_empty() {
        match socket.write(data) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => data = &data[n..],
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(ErrorKind::TimedOut.into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    let _ = socket.flush();
    Ok(())
}

/// Extrae líneas completas y las reparte. Guarda solo el último snapshot y
/// devuelve `false` si la UI desapareció o si el servidor pidió `Quit`.
fn drain_lines(
    buffer: &mut Vec<u8>,
    events: &Sender<NetEvent>,
    latest: &Arc<Mutex<Option<MiniSnapshot>>>,
) -> bool {
    while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=end).collect();
        let text = String::from_utf8_lossy(&line[..line.len().saturating_sub(1)]);
        let text = text.trim_end_matches('\r').trim();

        if text.is_empty() {
            continue;
        }

        if let Ok(snapshot) = MiniSnapshot::from_line(text) {
            if let Ok(mut slot) = latest.lock() {
                *slot = Some(snapshot);
            }
            continue;
        }

        // El servidor solo debería mandar snapshots, pero aceptamos `Quit`
        // como orden de cierre del cliente.
        if let Ok(command) = MiniCommand::from_line(text) {
            if command.is_quit() {
                let _ = events.send(NetEvent::Quit);
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    fn unique_socket() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "spote-mini-test-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    fn accept_with_timeout(listener: &UnixListener) -> UnixStream {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "el cliente no conectó");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("accept falló: {err}"),
            }
        }
    }

    fn wait_snapshot(link: &MiniLink) -> MiniSnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(snapshot) = link.take_snapshot() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "no llegó ningún snapshot");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_event(link: &MiniLink, wanted: NetEvent) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            while let Some(event) = link.try_recv() {
                if event == wanted {
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn shutdown_sends_quit_to_server() {
        let path = unique_socket();
        let listener = UnixListener::bind(&path).unwrap();
        let mut link = MiniLink::spawn(path.clone());
        let server = accept_with_timeout(&listener);
        assert!(wait_event(&link, NetEvent::Connected));

        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(server.try_clone().unwrap());
        link.shutdown();

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "\"Quit\"");

        drop(link);
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn socket_roundtrip_snapshot_command_and_quit() {
        let path = unique_socket();
        let listener = UnixListener::bind(&path).unwrap();
        let mut link = MiniLink::spawn(path.clone());
        let mut server = accept_with_timeout(&listener);

        let snapshot = MiniSnapshot::new("Tema", "Artista")
            .with_playing(true)
            .with_duration_ms(1_000)
            .with_bands([0.2, 0.3]);
        server
            .write_all(snapshot.to_line().unwrap().as_bytes())
            .unwrap();
        assert_eq!(wait_snapshot(&link), snapshot);
        assert!(wait_event(&link, NetEvent::Connected));

        link.send(MiniCommand::PlayPause);
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(server.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "\"PlayPause\"");

        server.write_all(b"\"Quit\"\n").unwrap();
        assert!(wait_event(&link, NetEvent::Quit));

        link.shutdown();
        drop(link);
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }
}
