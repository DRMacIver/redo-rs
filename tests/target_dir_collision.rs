/// Regression test: when a target name collides with a subdirectory name,
/// the target should not be incorrectly reported as a source file.
///
/// This tests the scenario where:
/// - Target "a/a" is built first, creating directory "a/"
/// - Target "a" is then built (fails because "a" is a directory)
/// - After the failed build, redo-sources should NOT list "a" as a source
///
/// The bug was that File entries created during a failed build were committed
/// to the database instead of being rolled back, causing them to appear as
/// source files with NULL stamps.
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

const RUST_REDO_BIN: &str = env!("CARGO_BIN_EXE_redo");
const PYTHON_REDO_DIR: &str = "/tmp/python-redo-bin";

fn rust_redo_dir() -> String {
    Path::new(RUST_REDO_BIN)
        .parent()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

fn ensure_rust_symlinks() {
    let dir = Path::new(RUST_REDO_BIN).parent().unwrap();
    let redo_bin = dir.join("redo");
    for cmd in &[
        "redo-ifchange", "redo-ifcreate", "redo-always", "redo-stamp",
        "redo-log", "redo-whichdo", "redo-targets", "redo-sources",
        "redo-ood", "redo-unlocked",
    ] {
        let link = dir.join(cmd);
        if !link.exists() {
            std::os::unix::fs::symlink(&redo_bin, &link).unwrap();
        }
    }
}

fn ensure_python_wrappers() {
    let dir = Path::new(PYTHON_REDO_DIR);
    if dir.join("redo").exists() {
        return;
    }
    std::fs::create_dir_all(dir).unwrap();
    let commands: &[(&str, &str)] = &[
        ("redo", "cmd_redo"), ("redo-ifchange", "cmd_ifchange"),
        ("redo-ifcreate", "cmd_ifcreate"), ("redo-always", "cmd_always"),
        ("redo-stamp", "cmd_stamp"), ("redo-log", "cmd_log"),
        ("redo-whichdo", "cmd_whichdo"), ("redo-targets", "cmd_targets"),
        ("redo-sources", "cmd_sources"), ("redo-ood", "cmd_ood"),
        ("redo-unlocked", "cmd_unlocked"),
    ];
    let version_dir = Path::new("/tmp/apenwarr-redo/redo/version");
    std::fs::create_dir_all(version_dir).unwrap();
    std::fs::write(
        version_dir.join("_version.py"),
        "COMMIT = 'test'\nTAG = 'test-0.0'\nDATE = '2024-01-01'\n",
    ).unwrap();
    for (cmd, module) in commands {
        let script = format!(
            "#!/usr/bin/env python3\nimport sys\nsys.path.insert(0, '/tmp/apenwarr-redo')\nfrom redo.{} import main\nmain()\n",
            module
        );
        let path = dir.join(cmd);
        std::fs::write(&path, &script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run_cmd(bin_dir: &str, cmd: &str, dir: &Path, args: &[&str]) -> (i32, String) {
    let mut command = Command::new(format!("{}/{}", bin_dir, cmd));
    command.args(args);
    command.current_dir(dir);
    command.env("REDO_LOG", "0");
    command.env("REDO_NO_OOB", "1");
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
    ] {
        command.env_remove(var);
    }
    command.env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin_dir));
    let output = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run command");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

fn setup_project(dir: &Path) {
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::write(dir.join("a.src"), "a\n").unwrap();

    // a/a.do: builds target a/a, depends on a.src with redo-stamp
    std::fs::write(
        dir.join("a/a.do"),
        concat!(
            "redo-ifchange a.src\n",
            "redo-always\n",
            "(\ncat a.src\necho built:a/a\n) | redo-stamp\n",
            "(\ncat a.src\necho built:a/a\n) >$3\n"
        ),
    ).unwrap();

    // a.do: builds target a (will fail because a/ is a directory), depends on a/a
    std::fs::write(
        dir.join("a.do"),
        concat!(
            "redo-ifchange a/a a.src\n",
            "redo-always\n",
            "(\ncat a.src\necho built:a\n) | redo-stamp\n",
            "(\ncat a.src\necho built:a\n) >$3\n"
        ),
    ).unwrap();
}

#[test]
fn test_sources_match_after_directory_target_collision() {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let base = std::env::temp_dir().join(format!("redo-dir-collision-{}", std::process::id()));
    let py_dir = base.join("python");
    let rs_dir = base.join("rust");

    std::fs::create_dir_all(&py_dir).unwrap();
    std::fs::create_dir_all(&rs_dir).unwrap();
    setup_project(&py_dir);
    setup_project(&rs_dir);

    // Build a/a then a (the latter will fail in both)
    run_cmd(PYTHON_REDO_DIR, "redo", &py_dir, &["a/a", "a"]);
    run_cmd(&rust_redo_dir(), "redo", &rs_dir, &["a/a", "a"]);

    // Compare redo-sources output
    let (py_rc, py_sources) = run_cmd(PYTHON_REDO_DIR, "redo-sources", &py_dir, &[]);
    let (rs_rc, rs_sources) = run_cmd(&rust_redo_dir(), "redo-sources", &rs_dir, &[]);

    let py_set: BTreeSet<&str> = py_sources.lines().collect();
    let rs_set: BTreeSet<&str> = rs_sources.lines().collect();

    assert_eq!(py_rc, rs_rc, "redo-sources exit code differs");
    assert_eq!(
        py_set, rs_set,
        "redo-sources output differs after directory/target collision\n\
         python: {:?}\n\
         rust:   {:?}\n\
         The Rust version should NOT list 'a' as a source since the \
         File entry should have been rolled back after the failed build.",
        py_set, rs_set,
    );

    let _ = std::fs::remove_dir_all(&base);
}
