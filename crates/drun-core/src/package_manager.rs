//! Everything specific to one package manager: how to invoke its installer,
//! where its installs live in the workspace, and what env var later shell
//! commands need to see them. The only place in the crate that knows pip
//! from npm — sandbox.rs and executor.rs stay entirely agnostic.

use crate::error::RunnerError;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackageManager {
    Pip,
    Npm,
}

impl PackageManager {
    const ALL: [PackageManager; 2] = [PackageManager::Pip, PackageManager::Npm];

    pub(crate) fn parse(name: &str) -> anyhow::Result<Self> {
        match name {
            "pip" => Ok(Self::Pip),
            "npm" => Ok(Self::Npm),
            other => Err(RunnerError::unsupported_package_manager(other).into()),
        }
    }

    /// Workspace-relative directory this manager's installs are merged into.
    pub(crate) fn install_dir(&self) -> &'static str {
        match self {
            Self::Pip => ".packages/pip",
            Self::Npm => ".packages/npm",
        }
    }

    /// Argv to run inside the install sandbox, targeting `staging_dir` (an
    /// absolute path — the disposable directory this run's installs land in
    /// before being merged into the checkpoint under `install_dir()`).
    pub(crate) fn install_argv(&self, staging_dir: &Path, packages: &[String]) -> Vec<String> {
        let target = staging_dir.display().to_string();
        let mut argv = match self {
            Self::Pip => vec![
                "python3".to_string(),
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "--no-input".to_string(),
                "--disable-pip-version-check".to_string(),
                "--target".to_string(),
                target,
            ],
            Self::Npm => vec![
                "npm".to_string(),
                "install".to_string(),
                "--no-audit".to_string(),
                "--no-fund".to_string(),
                "--prefix".to_string(),
                target,
            ],
        };
        argv.extend(packages.iter().cloned());
        argv
    }

    /// The workspace-relative path a later shell command needs on its search
    /// path (PYTHONPATH, NODE_PATH, ...) to use whatever this manager
    /// installed.
    fn search_path(&self) -> &'static str {
        match self {
            Self::Pip => ".packages/pip",
            Self::Npm => ".packages/npm/node_modules",
        }
    }

    fn env_var_name(&self) -> &'static str {
        match self {
            Self::Pip => "PYTHONPATH",
            Self::Npm => "NODE_PATH",
        }
    }

    /// Every package manager's search-path env var, rooted at `workspace`.
    /// Harmless to set unconditionally — a missing directory is a no-op for
    /// both PYTHONPATH and NODE_PATH.
    pub(crate) fn workspace_env_vars(workspace: &Path) -> Vec<(&'static str, PathBuf)> {
        Self::ALL
            .iter()
            .map(|pm| (pm.env_var_name(), workspace.join(pm.search_path())))
            .collect()
    }

    /// Rejects flag-injection (a spec starting with `-`) and shell
    /// metacharacters, while allowing every character real pip/npm
    /// specifiers use (version pins, extras, npm scopes like `@org/pkg`).
    pub(crate) fn validate_package_spec(pkg: &str) -> anyhow::Result<()> {
        let is_safe_char = |c: char| c.is_ascii_alphanumeric() || ".+_-@/=<>!~,[]".contains(c);
        let valid = !pkg.is_empty() && !pkg.starts_with('-') && pkg.chars().all(is_safe_char);
        if valid {
            Ok(())
        } else {
            Err(RunnerError::invalid_package_spec(format!(
                "'{pkg}' is not a valid package specifier"
            ))
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_recognizes_known_managers() {
        assert_eq!(PackageManager::parse("pip").unwrap(), PackageManager::Pip);
        assert_eq!(PackageManager::parse("npm").unwrap(), PackageManager::Npm);
    }

    #[test]
    fn parse_rejects_an_unknown_manager() {
        let err = PackageManager::parse("cargo").unwrap_err();
        assert!(err.to_string().contains("cargo"));
    }

    #[test]
    fn install_argv_targets_the_staging_dir_and_appends_packages() {
        let staging = Path::new("/tmp/staging");
        let argv = PackageManager::Pip.install_argv(staging, &["requests".to_string()]);
        assert!(argv.contains(&"--target".to_string()));
        assert!(argv.contains(&"/tmp/staging".to_string()));
        assert_eq!(argv.last().unwrap(), "requests");

        let argv = PackageManager::Npm.install_argv(staging, &["left-pad".to_string()]);
        assert!(argv.contains(&"--prefix".to_string()));
        assert_eq!(argv.last().unwrap(), "left-pad");
    }

    #[test]
    fn workspace_env_vars_covers_every_manager_rooted_at_the_workspace() {
        let vars = PackageManager::workspace_env_vars(Path::new("/workspace"));
        assert_eq!(
            vars,
            vec![
                ("PYTHONPATH", PathBuf::from("/workspace/.packages/pip")),
                (
                    "NODE_PATH",
                    PathBuf::from("/workspace/.packages/npm/node_modules")
                ),
            ]
        );
    }

    #[test]
    fn validate_package_spec_accepts_common_pip_and_npm_specifiers() {
        for spec in [
            "requests",
            "numpy==1.26.4",
            "package[extra]",
            "pkg>=1.0,<2.0",
            "left-pad@1.3.0",
            "@scope/package",
        ] {
            assert!(
                PackageManager::validate_package_spec(spec).is_ok(),
                "expected {spec:?} to be accepted"
            );
        }
    }

    #[test]
    fn validate_package_spec_rejects_flag_injection_and_shell_metacharacters() {
        for spec in [
            "--index-url=http://evil",
            "-e",
            "pkg; rm -rf /",
            "pkg && curl evil",
            "pkg $(whoami)",
            "",
        ] {
            assert!(
                PackageManager::validate_package_spec(spec).is_err(),
                "expected {spec:?} to be rejected"
            );
        }
    }
}
