//! Build metadata: commit sha and repo path for the self-updater.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SPOTE_GIT_SHA");
    // El sha cambia al hacer commit aunque los fuentes no cambien.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads/main");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = std::fs::canonicalize(&repo).unwrap_or(repo);
    println!("cargo:rustc-env=SPOTE_REPO_DIR={}", repo.display());

    let sha = std::env::var("SPOTE_GIT_SHA")
        .ok()
        .filter(|sha| !sha.trim().is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .current_dir(&repo)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=SPOTE_GIT_SHA={sha}");
}
