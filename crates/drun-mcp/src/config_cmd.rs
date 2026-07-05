use std::{
    path::{Path, PathBuf},
    process::Command,
};

use toml_edit::{Array, DocumentMut, Item, Value};

pub fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("add-domain") => match args.get(1) {
            Some(domain) => add_domain(domain),
            None => usage_and_exit("drun-mcp config add-domain <domain>"),
        },
        Some("add-path") => match args.get(1) {
            Some(path) => add_path(path),
            None => usage_and_exit("drun-mcp config add-path <path>"),
        },
        Some("remove-domain") => match args.get(1) {
            Some(domain) => remove_domain(domain),
            None => usage_and_exit("drun-mcp config remove-domain <domain>"),
        },
        Some("remove-path") => match args.get(1) {
            Some(path) => remove_path(path),
            None => usage_and_exit("drun-mcp config remove-path <path>"),
        },
        Some("list") => list(),
        _ => usage_and_exit(
            "drun-mcp config <add-domain|add-path|remove-domain|remove-path|list> [args]",
        ),
    }
}

fn usage_and_exit(usage: &str) -> ! {
    eprintln!("usage: {usage}");
    std::process::exit(1);
}

fn config_path() -> PathBuf {
    crate::init::drun_home().join("config.toml")
}

fn add_domain(domain: &str) {
    let path = config_path();
    match add_domain_to(&path, domain) {
        Ok(true) => {
            eprintln!(
                "drun: added '{domain}' to domain_allowlist in {}",
                path.display()
            );
            restart_daemon();
        }
        Ok(false) => eprintln!("drun: '{domain}' already in domain_allowlist, skipping"),
        Err(e) => {
            eprintln!("drun: {e}");
            std::process::exit(1);
        }
    }
}

fn add_path(path_arg: &str) {
    let abs = match Path::new(path_arg).canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("drun: cannot resolve path '{path_arg}': {e}");
            std::process::exit(1);
        }
    };
    let path = config_path();
    match add_path_to(&path, &abs) {
        Ok(true) => {
            eprintln!(
                "drun: added '{}' to mount_allowlist in {}",
                abs.display(),
                path.display()
            );
            restart_daemon();
        }
        Ok(false) => eprintln!(
            "drun: '{}' already in mount_allowlist, skipping",
            abs.display()
        ),
        Err(e) => {
            eprintln!("drun: {e}");
            std::process::exit(1);
        }
    }
}

fn remove_domain(domain: &str) {
    let path = config_path();
    match remove_domain_from(&path, domain) {
        Ok(true) => {
            eprintln!(
                "drun: removed '{domain}' from domain_allowlist in {}",
                path.display()
            );
            restart_daemon();
        }
        Ok(false) => eprintln!("drun: '{domain}' not in domain_allowlist, skipping"),
        Err(e) => {
            eprintln!("drun: {e}");
            std::process::exit(1);
        }
    }
}

fn remove_path(path_arg: &str) {
    // Unlike `add-path`, don't require the path to still exist on disk —
    // removing a stale entry for something that's since been deleted should
    // still work. Canonicalize on a best-effort basis so a path that *is*
    // still present matches however it was originally stored.
    let value = Path::new(path_arg)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path_arg.to_string());
    let path = config_path();
    match remove_path_from(&path, &value) {
        Ok(true) => {
            eprintln!(
                "drun: removed '{value}' from mount_allowlist in {}",
                path.display()
            );
            restart_daemon();
        }
        Ok(false) => eprintln!("drun: '{value}' not in mount_allowlist, skipping"),
        Err(e) => {
            eprintln!("drun: {e}");
            std::process::exit(1);
        }
    }
}

fn list() {
    let path = config_path();
    let config = drun_core::Config::load_from(Some(&path));
    println!("domain_allowlist:");
    for domain in &config.domain_allowlist {
        println!("  - {domain}");
    }
    println!("mount_allowlist:");
    if config.mount_allowlist.is_empty() {
        println!("  (empty; all paths permitted)");
    } else {
        for path in &config.mount_allowlist {
            println!("  - {}", path.display());
        }
    }
}

fn add_domain_to(config_path: &Path, domain: &str) -> Result<bool, String> {
    add_to_array(config_path, "domain_allowlist", domain)
}

fn add_path_to(config_path: &Path, path: &Path) -> Result<bool, String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()))?;
    add_to_array(config_path, "mount_allowlist", value)
}

fn add_to_array(config_path: &Path, key: &str, value: &str) -> Result<bool, String> {
    if !config_path.exists() {
        return Err(format!(
            "no config found at {} — run install.sh first",
            config_path.display()
        ));
    }
    let contents = std::fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
    let mut doc = contents
        .parse::<DocumentMut>()
        .map_err(|e| format!("cannot parse {}: {e}", config_path.display()))?;

    let already_present = doc
        .get(key)
        .and_then(Item::as_array)
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(value)));
    if already_present {
        return Ok(false);
    }

    let entry = doc
        .entry(key)
        .or_insert_with(|| Item::Value(Value::Array(Array::new())));
    let array = entry
        .as_array_mut()
        .ok_or_else(|| format!("'{key}' in {} is not an array", config_path.display()))?;
    array.push(value);

    std::fs::write(config_path, doc.to_string())
        .map_err(|e| format!("cannot write {}: {e}", config_path.display()))?;
    Ok(true)
}

fn remove_domain_from(config_path: &Path, domain: &str) -> Result<bool, String> {
    remove_from_array(config_path, "domain_allowlist", domain)
}

fn remove_path_from(config_path: &Path, path: &str) -> Result<bool, String> {
    remove_from_array(config_path, "mount_allowlist", path)
}

/// Removes the first entry equal to `value` from the array at `key`,
/// preserving every comment and formatting already in the file. Returns
/// `Ok(false)` without writing if `value` isn't present.
fn remove_from_array(config_path: &Path, key: &str, value: &str) -> Result<bool, String> {
    if !config_path.exists() {
        return Err(format!(
            "no config found at {} — run install.sh first",
            config_path.display()
        ));
    }
    let contents = std::fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
    let mut doc = contents
        .parse::<DocumentMut>()
        .map_err(|e| format!("cannot parse {}: {e}", config_path.display()))?;

    let Some(array) = doc.get_mut(key).and_then(Item::as_array_mut) else {
        return Ok(false);
    };
    let Some(idx) = array.iter().position(|v| v.as_str() == Some(value)) else {
        return Ok(false);
    };
    array.remove(idx);

    std::fs::write(config_path, doc.to_string())
        .map_err(|e| format!("cannot write {}: {e}", config_path.display()))?;
    Ok(true)
}

fn restart_daemon() {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::env::consts::OS {
        "macos" => {
            let plist = format!("{home}/Library/LaunchAgents/com.drun.mcp-server.plist");
            if !Path::new(&plist).exists() {
                eprintln!("drun: no launchd agent found at {plist} — restart the daemon manually");
                return;
            }
            let _ = Command::new("launchctl").args(["unload", &plist]).status();
            let status = Command::new("launchctl")
                .args(["load", "-w", &plist])
                .status();
            match status {
                Ok(s) if s.success() => eprintln!("drun: daemon restarted"),
                _ => eprintln!(
                    "drun: could not restart the daemon automatically — run:\n  launchctl unload {plist}\n  launchctl load -w {plist}"
                ),
            }
        }
        "linux" => {
            let status = Command::new("systemctl")
                .args(["--user", "restart", "drun-mcp.service"])
                .status();
            match status {
                Ok(s) if s.success() => eprintln!("drun: daemon restarted"),
                _ => eprintln!(
                    "drun: could not restart the daemon automatically — run:\n  systemctl --user restart drun-mcp.service"
                ),
            }
        }
        _ => eprintln!("drun: unsupported platform — restart the daemon manually"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sample_config(dir: &Path) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            "# drun configuration — all fields are optional; these are the defaults.\n\n\
             # Domains agents may reach via session_fetch.\n\
             domain_allowlist = [\"pypi.org\"]\n\n\
             # Host path prefixes agents may mount.\n\
             mount_allowlist = []\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn add_domain_appends_and_preserves_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());

        let added = add_domain_to(&path, "example.com").unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# drun configuration"));
        assert!(content.contains("# Domains agents may reach via session_fetch."));
        assert!(content.contains("pypi.org"));
        assert!(content.contains("example.com"));
    }

    #[test]
    fn added_domain_is_visible_through_config_load_from() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());

        add_domain_to(&path, "example.com").unwrap();

        let config = drun_core::Config::load_from(Some(&path));
        assert!(config.domain_allowlist.contains(&"example.com".to_string()));
    }

    #[test]
    fn add_domain_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());

        assert!(add_domain_to(&path, "example.com").unwrap());
        assert!(!add_domain_to(&path, "example.com").unwrap());

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches("example.com").count(), 1);
    }

    #[test]
    fn add_path_appends_to_mount_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());
        let mount_dir = tempfile::tempdir().unwrap();

        let added = add_path_to(&path, mount_dir.path()).unwrap();
        assert!(added);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(mount_dir.path().to_str().unwrap()));
    }

    #[test]
    fn add_path_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());
        let mount_dir = tempfile::tempdir().unwrap();

        assert!(add_path_to(&path, mount_dir.path()).unwrap());
        assert!(!add_path_to(&path, mount_dir.path()).unwrap());
    }

    #[test]
    fn remove_domain_deletes_the_entry_and_preserves_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());
        add_domain_to(&path, "example.com").unwrap();

        let removed = remove_domain_from(&path, "example.com").unwrap();
        assert!(removed);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# drun configuration"));
        assert!(content.contains("pypi.org"));
        assert!(!content.contains("example.com"));
    }

    #[test]
    fn remove_domain_is_a_no_op_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());

        assert!(!remove_domain_from(&path, "example.com").unwrap());
    }

    #[test]
    fn remove_path_deletes_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());
        let mount_dir = tempfile::tempdir().unwrap();
        let value = mount_dir.path().to_str().unwrap();
        add_path_to(&path, mount_dir.path()).unwrap();

        let removed = remove_path_from(&path, value).unwrap();
        assert!(removed);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(value));
    }

    #[test]
    fn remove_path_is_a_no_op_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_sample_config(dir.path());

        assert!(!remove_path_from(&path, "/nonexistent").unwrap());
    }

    #[test]
    fn remove_domain_errors_on_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let err = remove_domain_from(&path, "example.com").unwrap_err();
        assert!(err.contains("install.sh"));
    }

    #[test]
    fn add_domain_errors_on_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let err = add_domain_to(&path, "example.com").unwrap_err();
        assert!(err.contains("install.sh"));
    }

    #[test]
    fn add_domain_errors_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        assert!(add_domain_to(&path, "example.com").is_err());
    }

    #[test]
    fn add_domain_errors_when_key_is_not_an_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "domain_allowlist = \"not-an-array\"\n").unwrap();

        let err = add_domain_to(&path, "example.com").unwrap_err();
        assert!(err.contains("not an array"));
    }

    #[test]
    fn remove_domain_errors_on_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        assert!(remove_domain_from(&path, "example.com").is_err());
    }

    #[test]
    fn remove_domain_is_a_no_op_when_key_is_absent_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "# no domain_allowlist key at all\n").unwrap();

        assert!(!remove_domain_from(&path, "example.com").unwrap());
    }
}
