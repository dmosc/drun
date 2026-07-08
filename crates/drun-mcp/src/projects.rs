use std::path::PathBuf;

/// Lists all registered drun projects from `~/.drun/projects`.
/// With `--clean`, removes entries whose directories no longer exist on disk.
pub fn run(args: &[String]) {
    if args.contains(&"--clean".to_string()) {
        clean();
    } else {
        list();
    }
}

fn list() {
    let (active, missing) = read_registry_partitioned();

    if active.is_empty() && missing.is_empty() {
        println!("No projects registered. Run `drun init` in a project directory.");
        return;
    }

    let total = active.len() + missing.len();
    println!("Registered projects ({total}):\n");

    for path in &active {
        println!("  {path}  [active]");
    }
    for path in &missing {
        println!("  {path}  [missing]");
    }

    if !missing.is_empty() {
        println!(
            "\n{} missing. Run `drun projects --clean` to remove stale entries.",
            missing.len()
        );
    }
}

fn clean() {
    let registry = crate::init::drun_home().join("projects");
    if !registry.exists() {
        eprintln!("drun: no project registry found");
        return;
    }

    let content = std::fs::read_to_string(&registry).unwrap_or_default();
    let (active, removed): (Vec<&str>, Vec<&str>) = content
        .lines()
        .filter(|l| !l.is_empty())
        .partition(|l| PathBuf::from(l).exists());

    if removed.is_empty() {
        eprintln!("drun: no stale entries found");
        return;
    }

    let filtered: String = active.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&registry, filtered).expect("cannot update project registry");

    for path in &removed {
        eprintln!("drun: removed stale entry: {path}");
    }
    let n = removed.len();
    eprintln!(
        "drun: cleaned {n} stale entr{}",
        if n == 1 { "y" } else { "ies" }
    );
}

fn read_registry_partitioned() -> (Vec<String>, Vec<String>) {
    let registry = crate::init::drun_home().join("projects");
    let content = std::fs::read_to_string(&registry).unwrap_or_default();

    content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .partition(|l| PathBuf::from(l).exists())
}
