use drun_core::{Config, Session};

fn cfg() -> Config {
    Config {
        max_workspace_mb: None,
        max_checkpoints: None,
        mount_overlay_paths: vec![],
        ..Config::default()
    }
}

#[test]
fn using_drun_api_mount_modify_commit_updates_host_file_test() {
    let dir = tempfile::tempdir().unwrap();
    let host_file = dir.path().join("data.txt");
    std::fs::write(&host_file, b"original").unwrap();

    let mut s = Session::new(&cfg()).unwrap();
    s.mount(dir.path()).unwrap();
    s.write_file("data.txt", b"modified".to_vec()).unwrap();
    s.commit(None).unwrap();

    assert_eq!(std::fs::read(&host_file).unwrap(), b"modified");
}
