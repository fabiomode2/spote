//! spote — TUI de música. Punto de entrada y bucle de eventos.

mod app;
mod fetch;
mod mini;
mod system_media;
mod hires;
mod social;
mod spotify_jobs;
mod theme;
mod ui;
mod version;
mod viz;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use spote_core::artwork::ArtMode;
use spote_core::catalog;
use spote_core::config::{self, Config};
use spote_core::downloader::{ingest_dir, DownloadEvent, Downloader};
use spote_core::library::{Library, SourceStatus};
use spote_core::record;
use spote_core::spotify::SpotifyWatcher;
use spote_core::ToolStatus;
use spote_protocol::TrackRef;

use crate::app::{App, ToastKind};

#[derive(Debug, Default)]
struct Args {
    command: Option<String>,
    rest: Vec<String>,
    follow: bool,
    no_viz: bool,
    art: Option<String>,
    viz: Option<String>,
    fps: Option<u32>,
    download: bool,
    dry_run: bool,
    flac: bool,
    from_now: bool,
    seconds: Option<u64>,
    step: Option<u64>,
    discord_id: Option<String>,
    save_discord: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Self::default();
        let mut iter = std::env::args().skip(1).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--follow-spotify" | "--follow-system" | "--spotify" => args.follow = true,
                "--no-viz" => args.no_viz = true,
                "--download" => args.download = true,
                "--dry-run" => args.dry_run = true,
                "--flac" => args.flac = true,
                "--from-now" => args.from_now = true,
                "--seconds" => args.seconds = iter.next().and_then(|value| value.parse().ok()),
                "--step" => args.step = iter.next().and_then(|value| value.parse().ok()),
                "--id" => args.discord_id = iter.next(),
                "--save" => args.save_discord = true,
                "--art" => args.art = iter.next(),
                "--viz" => args.viz = iter.next(),
                "--fps" => args.fps = iter.next().and_then(|value| value.parse().ok()),
                "--version" | "-V" => args.command = Some("version".into()),
                "--help" | "-h" => args.command = Some("help".into()),
                other if other.starts_with('-') => {
                    eprintln!("spote: opción desconocida {other}");
                }
                other => {
                    if args.command.is_none() {
                        args.command = Some(other.to_string());
                    } else {
                        args.rest.push(other.to_string());
                    }
                }
            }
        }
        args
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("spote: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    match args.command.as_deref() {
        Some("ingest") => return run_ingest(&args),
        Some("import") => return run_import(&args),
        Some("record") => return run_record(&args),
        Some("harvest") => return run_harvest(&args),
        Some("discord") => return run_discord(&args),
        Some("mini") => return run_mini(),
        Some("doctor") => return run_doctor(),
        Some("version") => {
            println!("spote {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("help") => {
            print_help();
            return Ok(());
        }
        Some(other) => {
            eprintln!("spote: comando desconocido «{other}»\n");
            print_help();
            return Ok(());
        }
        None => {}
    }
    run_tui(args)
}

fn run_mini() -> Result<()> {
    let exe = std::env::current_exe()?;
    let path = exe
        .parent()
        .map(|dir| dir.join("spote-mini"))
        .filter(|path| path.is_file());
    let mut command = if let Some(path) = path {
        std::process::Command::new(path)
    } else {
        std::process::Command::new("spote-mini")
    };
    let status = command.status().context("no pude abrir spote-mini")?;
    if !status.success() {
        anyhow::bail!("spote-mini terminó con {status}");
    }
    Ok(())
}

fn print_help() {
    println!(
        "spote {} — reproductor de música con TUI

USO:
  spote [opciones]                 abre la TUI
  spote ingest <dir>               indexa audio local (ogg/opus/flac/mp3/m4a)
  spote import <url-spotify>       importa playlist/álbum/artista/pista
         [--download] [--dry-run]   y opcionalmente baja el audio con yt-dlp
  spote record                     graba lo que suena (exclusivas, podcasts)
         [--flac] [--from-now] [--seconds N]
  spote harvest [<url>]            saca el audio descifrado de la memoria del
         [--step ms]               cliente saltando por la pista (2-4× más rápido)
  spote discord --id <app-id>      prueba el Rich Presence y guarda el ID
         [--save] [--seconds N]
  spote mini                       abre el mini reproductor de escritorio
  spote doctor                     comprueba dependencias y rutas
  spote version | help

OPCIONES:
  --follow-system                  sigue el reproductor MPRIS del sistema
  --follow-spotify                 alias de --follow-system
  --art <default|pixel|braille|off>
  --viz <preset>                   clean, aurora, bloom, inferno, crt,
                                   lcd, rack, braille, stage (franja)
  --no-viz                         arranca sin visualizador
  --fps <n>                        límite de frames de la UI (20-60, default 60)

DATOS:
  db      {}
  media   {}
  config  {}
",
        env!("CARGO_PKG_VERSION"),
        config::db_path().display(),
        Config::load().media_dir().display(),
        config::config_path().display()
    );
}

fn run_ingest(args: &Args) -> Result<()> {
    let Some(dir) = args.rest.first() else {
        anyhow::bail!("uso: spote ingest <directorio>");
    };
    let mut cfg = Config::load();
    if let Some(art) = &args.art {
        cfg.ui.art_mode = art.clone();
    }
    let mut lib = Library::open(&config::db_path())?;
    let path = PathBuf::from(dir);
    println!("indexando {} …", path.display());
    let report = ingest_dir(&mut lib, &path, &cfg.media_dir())?;
    println!(
        "importadas {} pistas ({} errores)",
        report.imported,
        report.errors.len()
    );
    for error in report.errors.iter().take(10) {
        println!("  ! {error}");
    }
    println!("librería: {} pistas", lib.count().unwrap_or(0));
    Ok(())
}

/// Importa una colección pública de Spotify (playlist, álbum o pista).
///
/// El catálogo se lee de la página `/embed/`, que no pide cuenta ni token.
/// Con `--download` se resuelve cada pista contra yt-dlp y se baja el audio.
fn run_import(args: &Args) -> Result<()> {
    let Some(target) = args.rest.first() else {
        anyhow::bail!("uso: spote import <url-o-uri-de-spotify> [--download] [--dry-run]");
    };

    println!("resolviendo catálogo de Spotify …");
    let catalog = catalog::fetch(target)?;
    let refs = catalog.to_track_refs();
    let owner = catalog
        .owner
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| format!(" · {value}"))
        .unwrap_or_default();
    println!(
        "{} «{}»{} — {} pistas",
        catalog.kind,
        catalog.name,
        owner,
        refs.len()
    );
    if catalog.kind == "playlist" && refs.len() >= 50 {
        println!("  aviso: el embed público devuelve 50 pistas como máximo; si la playlist es más larga falta el resto");
    }

    if args.dry_run {
        for (index, track) in refs.iter().enumerate() {
            println!(
                "  {:>3}. {:<58} {}",
                index + 1,
                track.label(),
                fmt_mmss(track.duration_ms)
            );
        }
        return Ok(());
    }

    let mut lib = Library::open(&config::db_path())?;
    // Un artista no es una playlist: sólo interesan sus temas.
    if catalog.kind == "artist" {
        for track in &refs {
            lib.upsert_track(track)?;
        }
        println!("indexadas {} pistas de «{}»", refs.len(), catalog.name);
    } else {
        let playlist_id = match lib.playlists()?.into_iter().find(|p| p.name == catalog.name) {
            Some(existing) => existing.id,
            None => lib.create_playlist(&catalog.name)?,
        };
        for track in &refs {
            lib.upsert_track(track)?;
            lib.add_to_playlist(playlist_id, &track.key)?;
        }
        println!(
            "indexadas {} pistas en la playlist local «{}»",
            refs.len(),
            catalog.name
        );
    }

    if !args.download {
        println!("(sin --download no se baja audio)");
        return Ok(());
    }
    download_refs(&mut lib, &refs)
}

/// Graba la salida del sistema para lo que no existe en ninguna otra fuente:
/// exclusivas de Spotify, podcasts, audiolibros. 1× tiempo real.
fn run_record(args: &Args) -> Result<()> {
    use record::RecordFormat;
    use std::io::Write;

    let media = Config::load().media_dir();
    let mut options = record::RecordOptions::new(media.clone());
    options.format = if args.flac {
        RecordFormat::Flac
    } else {
        RecordFormat::Opus
    };
    options.from_start = !args.from_now;
    options.seconds = args.seconds;

    println!("preparando la grabación …");
    let recorder = record::Recorder::start(&options)?;
    let track = recorder.track().clone();
    let total_ms = options.seconds.map(|s| s * 1000).unwrap_or(track.duration_ms);
    println!(
        "grabando «{}» ({}) — 1× tiempo real; Ctrl-C corta y deja lo grabado",
        track.label(),
        if total_ms > 0 {
            fmt_mmss(total_ms)
        } else {
            "--:--".to_string()
        }
    );

    let mut watcher = SpotifyWatcher::new();
    let mut last_tick = 0u64;
    let mut stalled_ms = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let elapsed_ms = recorder.elapsed().as_millis() as u64;
        if elapsed_ms / 1000 > last_tick {
            last_tick = elapsed_ms / 1000;
            print!(
                "\r  {} / {}   ",
                fmt_mmss(elapsed_ms),
                if total_ms > 0 {
                    fmt_mmss(total_ms)
                } else {
                    "--:--".to_string()
                }
            );
            let _ = std::io::stdout().flush();
        }

        // Margen: parec tarda ~1-2 s en arrancar y eso es silencio al principio.
        // Con pista completa corta antes el cambio de MPRIS; con `--seconds`
        // hay que dejar aire para que el recorte final tenga los segundos pedidos.
        if args
            .seconds
            .is_some_and(|limit| elapsed_ms >= limit * 1000 + 3_000)
        {
            break;
        }
        if track.duration_ms > 0 && elapsed_ms > track.duration_ms + 2_500 {
            break;
        }
        match watcher.poll() {
            Some(state) => {
                let same_track = state
                    .track
                    .as_ref()
                    .map(|current| current.key == track.key)
                    .unwrap_or(false);
                if state.track.is_some() && !same_track {
                    println!("\n  cambió la pista: corto aquí");
                    break;
                }
                if state.playing {
                    stalled_ms = 0;
                } else {
                    stalled_ms += 200;
                    if stalled_ms > 3_000 {
                        println!("\n  pausado: corto aquí");
                        break;
                    }
                }
            }
            None => {
                stalled_ms += 200;
                if stalled_ms > 5_000 {
                    break;
                }
            }
        }
    }
    println!();

    let recording = recorder.finish(options.format, &media)?;

    // Misma clave que el catálogo: si la pista venía de `spote import`, esta
    // grabación la convierte de pendiente en reproducible.
    let mut lib = Library::open(&config::db_path())?;
    lib.upsert_track(&recording.track)?;
    if let Some(row) = lib.track_by_key(&recording.track.key)? {
        let size = std::fs::metadata(&recording.path).ok().map(|meta| meta.len());
        lib.set_source(
            row.id,
            "recording",
            None,
            Some(&recording.path),
            SourceStatus::Ready,
            size,
        )?;
    }

    let size = std::fs::metadata(&recording.path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    println!(
        "guardado: {} ({:.0}s, {:.1} MB)",
        recording.path.display(),
        recording.seconds,
        size as f64 / 1_048_576.0
    );
    match recording.peak_db {
        Some(peak) => println!(
            "  pico {peak:.1} dBFS, ganancia aplicada {gain:+.1} dB",
            gain = recording.gain_db
        ),
        None => {}
    }
    if args.seconds.is_none() && recording.track.duration_ms > 0 {
        let expected = recording.track.duration_ms as f64 / 1000.0;
        let diff = recording.seconds - expected;
        if diff.abs() > 1.0 {
            println!(
                "  aviso: {diff:+.1}s respecto a la duración de la pista ({expected:.0}s)"
            );
        }
    }
    Ok(())
}

/// Saca el audio ya descifrado de la memoria del cliente saltando por la pista.
///
/// El cliente descifra justo a tiempo, así que no hay buffer que adelantar: se
/// le fuerza a descifrar cada posición con `SetPosition` y se recogen las
/// páginas Ogg (~2 s por salto) antes de que las libere. 2-4× tiempo real y sin
/// reencodar: el `.ogg` es el Vorbis original del CDN.
fn run_harvest(args: &Args) -> Result<()> {
    use spote_core::harvest;
    use std::io::Write;

    let media = Config::load().media_dir();
    let mut options = harvest::HarvestOptions::new(media.clone());
    options.target = args.rest.first().cloned();
    if let Some(step) = args.step {
        options.step_ms = step.max(250);
    }
    if let Some(limit) = args.seconds {
        options.budget = Duration::from_secs(limit);
    }

    println!("extrayendo del cliente …");
    let report = harvest::harvest(&options, |progress| {
        print!(
            "\r  {:>4.0}s / {:.0}s   {} páginas   {:.1}%   {:.2}x   ",
            progress.position_ms as f64 / 1000.0,
            progress.total_ms as f64 / 1000.0,
            progress.pages,
            progress.coverage,
            progress.speed
        );
        let _ = std::io::stdout().flush();
    })?;
    println!();

    let duration = spote_core::record::probe_duration(&report.path);
    println!(
        "guardado: {} ({} páginas, cobertura {:.1}%, {:.0}s, {:.2}x)",
        report.path.display(),
        report.pages,
        report.coverage,
        report.seconds,
        report.speed
    );
    if let Some(duration) = duration {
        let expected = report.track.duration_ms as f64 / 1000.0;
        println!(
            "  duración {duration:.1}s (pista {expected:.1}s, {diff:+.1}s)",
            diff = duration - expected
        );
    }

    let mut lib = Library::open(&config::db_path())?;
    lib.upsert_track(&report.track)?;
    if let Some(row) = lib.track_by_key(&report.track.key)? {
        let size = std::fs::metadata(&report.path).ok().map(|meta| meta.len());
        lib.set_source(
            row.id,
            "harvest",
            None,
            Some(&report.path),
            SourceStatus::Ready,
            size,
        )?;
    }
    Ok(())
}

/// Resuelve y descarga cada pista con el mismo pipeline que la TUI.
fn download_refs(lib: &mut Library, refs: &[TrackRef]) -> Result<()> {
    let media = Config::load().media_dir();
    let downloader = Downloader::start();
    let (mut done, mut skipped, mut failed) = (0usize, 0usize, 0usize);

    for (index, track) in refs.iter().enumerate() {
        let position = format!("[{}/{}]", index + 1, refs.len());
        if lib.local_path(&track.key).is_some() {
            skipped += 1;
            println!("{position} ya está: {}", track.label());
            continue;
        }

        println!("{position} ↓ {}", track.label());
        downloader.resolve_and_download(
            &track.key,
            &track.artist,
            &track.title,
            track.duration_ms,
            media.clone(),
        );
        match wait_download(&downloader, &track.key, Duration::from_secs(600)) {
            Ok(path) => {
                if let Ok(Some(row)) = lib.track_by_key(&track.key) {
                    lib.set_source(
                        row.id,
                        "ytmusic",
                        None,
                        Some(&path),
                        SourceStatus::Ready,
                        std::fs::metadata(&path).ok().map(|meta| meta.len()),
                    )?;
                }
                done += 1;
            }
            Err(err) => {
                failed += 1;
                println!("{position} error: {err}");
            }
        }
    }

    println!("\ndescargadas {done}, ya estaban {skipped}, fallidas {failed}");
    Ok(())
}

fn wait_download(downloader: &Downloader, key: &str, timeout: Duration) -> Result<PathBuf> {
    use std::io::Write;
    let start = Instant::now();
    let mut painted = String::new();
    while start.elapsed() < timeout {
        match downloader.try_recv() {
            Some(DownloadEvent::Progress {
                key: k,
                percent,
                stage,
            }) if k == key => {
                let line = format!("{stage} {percent:.0}%");
                if line != painted {
                    print!("\r    {line:<28}");
                    let _ = std::io::stdout().flush();
                    painted = line;
                }
            }
            Some(DownloadEvent::Done { key: k, path }) if k == key => {
                if !painted.is_empty() {
                    println!();
                }
                return Ok(path);
            }
            Some(DownloadEvent::Failed { key: k, message }) if k == key => {
                if !painted.is_empty() {
                    println!();
                }
                return Err(anyhow::anyhow!(message));
            }
            Some(_) => continue,
            None => std::thread::sleep(Duration::from_millis(80)),
        }
    }
    Err(anyhow::anyhow!("timeout esperando la descarga"))
}

fn fmt_mmss(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

fn run_doctor() -> Result<()> {
    let tools = ToolStatus::detect();
    println!("spote {}", env!("CARGO_PKG_VERSION"));
    println!("\nbinarios:");
    for (name, present) in [
        ("mpv", tools.mpv),
        ("ffmpeg", tools.ffmpeg),
        ("yt-dlp", tools.yt_dlp),
        ("parec", tools.parec),
    ] {
        println!("  {:<8} {}", name, if present { "ok" } else { "FALTA" });
    }
    if spote_core::util::which("mpv").is_some() {
        if let Ok(output) = std::process::Command::new("mpv")
            .arg("--version")
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = text.lines().next() {
                println!("  mpv: {}", first.trim());
            }
        }
    }
    if spote_core::stream::ytdlp_available() {
        if let Ok(output) = std::process::Command::new(spote_core::stream::ytdlp_bin())
            .arg("--version")
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(first) = text.lines().next() {
                println!("  yt-dlp: {}", first.trim());
            }
        }
    }
    println!("\nrutas:");
    println!("  db      {}", config::db_path().display());
    println!("  media   {}", Config::load().media_dir().display());
    println!("  config  {}", config::config_path().display());
    println!("  cache   {}", config::cache_dir().display());
    let lib = Library::open(&config::db_path())?;
    println!(
        "\nlibrería: {} pistas, {} descargadas",
        lib.count().unwrap_or(0),
        lib.ready_count().unwrap_or(0)
    );
    println!(
        "mpris: {}",
        if spote_core::spotify::SpotifyWatcher::session_bus_available() {
            "bus de sesión disponible"
        } else {
            "sin bus de sesión"
        }
    );
    Ok(())
}

fn run_tui(args: Args) -> Result<()> {
    // Log a fichero: nunca a la terminal (rompería el render).
    init_tracing();

    let mut cfg = Config::load();
    if args.follow {
        cfg.ui.follow_spotify = true;
    }
    if args.no_viz {
        cfg.ui.viz_enabled = false;
    }
    if let Some(art) = args.art {
        cfg.ui.art_mode = art;
    }
    if let Some(viz) = args.viz {
        cfg.ui.viz = viz;
    }
    if let Some(fps) = args.fps {
        cfg.ui.fps = fps.clamp(20, 60);
    }
    if ArtMode::from_str(&cfg.ui.art_mode).is_none() {
        cfg.ui.art_mode = "plain".into();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("no se pudo crear el runtime")?;

    let mut app = App::new(cfg.clone())?;
    app.start_mpris();
    match runtime.block_on(app.connect_player()) {
        Ok(()) => {}
        Err(err) => {
            app.toast(format!("mpv no disponible: {err}"), ToastKind::Error);
        }
    }

    let mut stdout = std::io::stdout();
    enable_raw_mode().context("no se pudo activar el modo raw")?;
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide,
        crossterm::terminal::SetTitle(cfg.app.name.clone())
    )?;
    // Las teclas con modificadores (shift+flechas) se anuncian si el terminal
    // soporta el protocolo; si no, crossterm lo ignora.
    let _ = execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        hook(info);
    }));

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Por encima de 60 fps el terminal no muestra nada nuevo y el coste por
    // frame (buffer completo + salida) se multiplica: una TUI de música no
    // necesita más.
    let fps = cfg.ui.fps.clamp(20, 60) as f32;
    let frame_budget = Duration::from_secs_f32(1.0 / fps);
    let mut last = Instant::now();
    let result = event_loop(&mut terminal, &mut app, frame_budget, &mut last);
    let _ = restore_terminal();
    if app.update_finished == Some(true) {
        println!("· spote actualizado · vuelve a abrirlo para usar la versión nueva");
    }
    result
}

/// Ritmo del bucle cuando nada anima (efectos en pausa incluidos).
const IDLE_POLL: Duration = Duration::from_millis(100);

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    budget: Duration,
    last: &mut Instant,
) -> Result<()> {
    let mut hires = hires::HiResState::default();
    let mut stdout = std::io::stdout();
    loop {
        let now = Instant::now();
        let dt = now.duration_since(*last).as_secs_f32().min(0.1);
        *last = now;

        app.tick(dt);
        if app.needs_redraw {
            app.needs_redraw = false;
            terminal.clear()?;
            hires.reset(&mut stdout);
        }
        let animating = app.animating();
        terminal.draw(|frame| {
            let area = frame.area();
            let buffer = frame.buffer_mut();
            ui::draw(app, buffer, area);
        })?;
        hires::render(&mut stdout, app, &mut hires);
        if app.should_quit {
            return Ok(());
        }

        // Con animación, al ritmo de `fps`; parado, a 10 Hz (los efectos en
        // pausa van a ese ritmo). Antes se repintaba a 60-120 Hz siempre,
        // incluso con la UI congelada.
        let wait = if animating {
            budget.saturating_sub(last.elapsed())
        } else {
            IDLE_POLL
        };
        if event::poll(wait)? {
            loop {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == crossterm::event::KeyEventKind::Press {
                            app.on_key(key);
                        }
                    }
                    Event::Mouse(mouse) => app.on_mouse(mouse),
                    Event::Resize(_, _) => {
                        app.art_key.clear();
                        app.request_redraw();
                    }
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
}

fn init_tracing() {
    let path = config::cache_dir().join("spote.log");
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("/tmp")));
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("SPOTE_LOG")
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_ansi(false)
            .try_init();
    }
}

fn restore_terminal() -> Result<()> {
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    execute!(
        stdout,
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::Show
    )?;
    disable_raw_mode()?;
    Ok(())
}

/// Prueba el Rich Presence: conecta, manda la actividad y la mantiene viva unos
/// segundos (al salir, Discord la borra).
fn run_discord(args: &Args) -> Result<()> {
    let id = args
        .discord_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let from_config = Config::load().discord.client_id;
            (!from_config.trim().is_empty()).then_some(from_config)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "uso: spote discord --id <application-id>\n                 (o pon el ID en [discord] client_id de config.toml)"
            )
        })?;

    if args.save_discord {
        let mut cfg = Config::load();
        cfg.discord.client_id = id.trim().to_string();
        cfg.discord.enabled = true;
        cfg.save().context("no pude guardar config.toml")?;
        println!("id guardado en {}", config::config_path().display());
    }
    println!("conectando con Discord (app {id}) …");
    let mut ipc = spote_core::discord::DiscordIpc::connect(&id)?;


    let mut watcher = SpotifyWatcher::new();
    let activity = match watcher.poll().and_then(|state| state.track) {
        Some(track) => {
            let mut state = track.artist.clone();
            if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
                state = format!("{state} · {album}");
            }
            spote_core::discord::Activity {
                details: track.title.clone(),
                state,
                start_ms: Some(
                    (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|elapsed| elapsed.as_millis() as i64)
                        .unwrap_or(0))
                        - track.duration_ms as i64 / 3,
                ),
                end_ms: None,
                large_image: track.art_url.clone(),
                large_text: None,
                small_image: None,
                small_text: Some("spote".into()),
                buttons: track
                    .key
                    .strip_prefix("spotify:track:")
                    .map(|id| vec![(
                        "Abrir en Spotify".to_string(),
                        format!("https://open.spotify.com/track/{id}"),
                    )])
                    .unwrap_or_default(),
            }
        }
        None => spote_core::discord::Activity {
            details: "Prueba de spote".into(),
            state: "Rich Presence funcionando".into(),
            start_ms: None,
            end_ms: None,
            large_image: None,
            large_text: None,
            small_image: None,
            small_text: Some("spote".into()),
            buttons: vec![],
        },
    };
    ipc.set_activity(&activity)?;
    println!("enviado: {} — {}", activity.details, activity.state);

    let seconds = args.seconds.unwrap_or(12);
    println!("manteniendo {seconds}s (mira tu perfil de Discord)…");
    std::thread::sleep(Duration::from_secs(seconds));
    ipc.clear().ok();
    Ok(())
}
