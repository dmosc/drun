use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Updates the drun binary in-place and re-initializes all registered projects.
///
/// Default behavior (no flags):
///   - downloads the latest (or pinned) release binary
///   - replaces the running binary atomically
///   - restarts the daemon
///   - silently re-runs `drun init` for every path in ~/.drun/projects
///
/// `--skip-reinit` skips the project re-initialization step.
/// `--version <tag>` pins to a specific release tag (e.g. "v0.4.0").
pub fn run(args: &[String]) {
    let skip_reinit = args.contains(&"--skip-reinit".to_string());
    let version = version_from_args(args);

    // snapshot the registry before the binary swap so we can re-init afterward
    // even if the update changes the registry location or format
    let registered: Vec<PathBuf> = if !skip_reinit {
        read_registry()
    } else {
        vec![]
    };

    let bin_path = current_binary_path();
    update_binary(&bin_path, &version);
    crate::config_cmd::restart_daemon();

    if !skip_reinit {
        reinit_projects(&registered);
    }

    eprintln!("drun: update complete");
}

fn version_from_args(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--version")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "latest".to_string())
}

fn current_binary_path() -> PathBuf {
    std::env::current_exe().expect("cannot determine path of the running binary")
}

fn detect_asset() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "drun-macos-arm64",
        ("linux", "x86_64") => "drun-linux-x86_64",
        (os, arch) => {
            eprintln!("drun: unsupported platform {os}/{arch}");
            std::process::exit(1);
        }
    }
}

/// Downloads the release binary to a temp path beside the current binary,
/// then atomically replaces it. Falls back to sudo if the directory isn't
/// writable by the current user.
fn update_binary(bin_path: &Path, version: &str) {
    let asset = detect_asset();
    let url = if version == "latest" {
        format!(
            "https://github.com/dmosc/drun/releases/latest/download/{asset}"
        )
    } else {
        format!(
            "https://github.com/dmosc/drun/releases/download/{version}/{asset}"
        )
    };

    eprintln!("drun: downloading {version}...");

    // Write to a temp file in the same directory so rename is atomic
    let tmp_path = bin_path.with_extension("tmp");
    let tmp_str = tmp_path.to_string_lossy();

    let download_ok = if command_exists("curl") {
        Command::new("curl")
            .args(["-fsSL", &url, "-o", &tmp_str])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else if command_exists("wget") {
        Command::new("wget")
            .args(["-qO", &tmp_str, &url])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        eprintln!(
            "drun: neither curl nor wget found — update manually:\n  curl -fsSL {url} -o {}",
            bin_path.display()
        );
        std::process::exit(1);
    };

    if !download_ok {
        eprintln!("drun: download failed");
        let _ = std::fs::remove_file(&tmp_path);
        std::process::exit(1);
    }

    // chmod +x on the downloaded binary
    #[cfg(unix)]
    set_executable(&tmp_path);

    // Atomic replace; fall back to sudo mv if the directory isn't writable
    if std::fs::rename(&tmp_path, bin_path).is_err() {
        eprintln!("drun: cannot replace binary directly, trying sudo...");
        let ok = Command::new("sudo")
            .args(["mv", &tmp_str, &bin_path.to_string_lossy()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !ok {
            eprintln!(
                "drun: failed to replace binary — update manually:\n  sudo mv {tmp_str} {}",
                bin_path.display()
            );
            let _ = std::fs::remove_file(&tmp_path);
            std::process::exit(1);
        }
    }

    eprintln!("drun: binary updated at {}", bin_path.display());
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn read_registry() -> Vec<PathBuf> {
    let registry = crate::init::drun_home().join("projects");
    std::fs::read_to_string(&registry)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn reinit_projects(registered: &[PathBuf]) {
    if registered.is_empty() {
        return;
    }

    eprintln!("drun: re-initializing {} project(s)...", registered.len());
    for project_dir in registered {
        if project_dir.exists() {
            crate::init::init_project(project_dir);
        } else {
            eprintln!(
                "drun: skipping missing directory {}",
                project_dir.display()
            );
        }
    }
}
