//! Comprobación de versión contra GitHub y lanzamiento del actualizador.
//!
//! El binario lleva el sha corto con el que se compiló. Al arrancar se pide
//! el último commit de `main`; si no coincide, la TUI ofrece actualizar. El
//! trabajo pesado lo hace `scripts/install.sh --update`, que hace `git pull`,
//! recompila e instala; aquí solo se lanza en segundo plano y se avisa al
//! terminar.
//!
//! La comprobación es opcional y silenciosa: sin red, sin git o sin script,
//! la app arranca igual. `SPOTE_NO_UPDATE_CHECK=1` la apaga.

use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub const LOCAL_SHA: &str = env!("SPOTE_GIT_SHA");
const REPO: &str = "fabiomode2/spote";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub latest: String,
}

/// Lanza la comprobación en un hilo y devuelve el receptor para el `tick`.
pub fn spawn_check() -> Option<Receiver<Option<UpdateInfo>>> {
    if std::env::var_os("SPOTE_NO_UPDATE_CHECK").is_some() || LOCAL_SHA.is_empty() {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("spote-update-check".into())
        .spawn(move || {
            let _ = tx.send(check());
        })
        .ok()?;
    Some(rx)
}

fn check() -> Option<UpdateInfo> {
    let latest = git_remote_sha().or_else(atom_feed_sha)?;
    if latest.starts_with(LOCAL_SHA) {
        return None;
    }
    Some(UpdateInfo { latest })
}

/// `git ls-remote` no pasa por la API (que limita por IP) y ya está git en
/// cualquier máquina que compile spote.
fn git_remote_sha() -> Option<String> {
    let output = Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "credential.helper=",
            "ls-remote",
            &format!("https://github.com/{REPO}.git"),
            "refs/heads/main",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let sha = text.split_whitespace().next()?;
    valid_sha(sha)
}

/// Sin git: el feed Atom público de commits, sin token ni límite de API.
fn atom_feed_sha() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("spote/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let text = client
        .get(format!("https://github.com/{REPO}/commits/main.atom"))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .ok()?;
    let marker = "Grit::Commit/";
    let start = text.find(marker)? + marker.len();
    let sha = text.get(start..start + 40)?;
    valid_sha(sha)
}

fn valid_sha(sha: &str) -> Option<String> {
    (sha.len() == 40 && sha.chars().all(|char| char.is_ascii_hexdigit()))
        .then(|| sha.to_string())
}

/// Arranca `scripts/install.sh --update --yes` y avisa con `true`/`false`.
pub fn spawn_update() -> Option<Receiver<bool>> {
    let script = std::path::Path::new(env!("SPOTE_REPO_DIR")).join("scripts/install.sh");
    if !script.is_file() {
        return None;
    }
    let log = spote_core::config::cache_dir().join("update.log");
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("spote-update".into())
        .spawn(move || {
            let mut command = Command::new("bash");
            command.arg(&script).arg("--update").arg("--yes");
            let status = match std::fs::File::create(&log) {
                Ok(file) => {
                    let errors = file.try_clone().ok();
                    command.stdout(Stdio::from(file));
                    if let Some(errors) = errors {
                        command.stderr(Stdio::from(errors));
                    }
                    command.status()
                }
                Err(_) => command.status(),
            };
            let _ = tx.send(status.map(|status| status.success()).unwrap_or(false));
        })
        .ok()?;
    Some(rx)
}
