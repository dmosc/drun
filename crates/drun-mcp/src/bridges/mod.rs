pub mod claude;
pub mod hermes;

/// Where a bridge's registration takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Scoped to the current working directory. Safe to run once per
    /// project without affecting drun's behavior in any other project.
    Project,
    /// Affects every session on this machine, regardless of project.
    Machine,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Machine => "machine",
        }
    }
}

/// A single agentic coding tool drun can wire itself into.
///
/// Implementations live in their own module (see `claude`, `hermes`) and are
/// exposed on the CLI by name — `drun-mcp <name> init` / `drun-mcp <name>
/// deregister` — via `REGISTRY` below, which both `main`'s dispatch and
/// `drun-mcp bridges list`/`deregister-all` read generically. Adding a new
/// bridge (Gemini, Codex, ...) means adding a new module that implements this
/// trait and one line in `REGISTRY` — no other file needs to change.
pub trait Bridge {
    /// Stable identifier used on the CLI (`drun-mcp <name> init`).
    fn name(&self) -> &'static str;

    /// One-line summary shown in `--help` and `drun-mcp bridges list`.
    fn description(&self) -> &'static str;

    fn scope(&self) -> Scope;

    /// Wire drun into this bridge. Must be idempotent: safe to call
    /// repeatedly, including once per project for a `Scope::Project` bridge.
    fn init(&self);

    /// Undo `init`. Must be safe to call even if never registered.
    fn deregister(&self);
}

/// Every bridge drun knows about. Display and dispatch order.
pub const REGISTRY: &[&dyn Bridge] = &[&claude::Claude, &hermes::Hermes];

/// Looks up a bridge by its CLI name (`drun-mcp <name> ...`).
pub fn find(name: &str) -> Option<&'static dyn Bridge> {
    REGISTRY.iter().copied().find(|b| b.name() == name)
}

/// Best-effort `deregister()` across every known bridge — used by
/// `uninstall.sh` so it doesn't need to name providers individually.
pub fn deregister_all() {
    for bridge in REGISTRY {
        bridge.deregister();
    }
}

/// Human-readable listing for `drun-mcp bridges list`. Printed to stdout
/// (unlike `init`/`deregister`'s progress messages on stderr) since this is
/// query output meant to be scriptable — e.g. `install.sh` pipes it directly
/// into its closing summary.
pub fn print_list() {
    let width = REGISTRY.iter().map(|b| b.name().len()).max().unwrap_or(0);
    for bridge in REGISTRY {
        println!(
            "{:<width$}  [{:<7}]  {}",
            bridge.name(),
            bridge.scope().label(),
            bridge.description()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_are_unique() {
        let mut names: Vec<&str> = REGISTRY.iter().map(|b| b.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate bridge name in REGISTRY");
    }

    #[test]
    fn find_locates_every_registered_bridge_by_name() {
        for bridge in REGISTRY {
            assert_eq!(find(bridge.name()).map(|b| b.name()), Some(bridge.name()));
        }
    }

    #[test]
    fn find_returns_none_for_an_unknown_name() {
        assert!(find("does-not-exist").is_none());
    }
}
