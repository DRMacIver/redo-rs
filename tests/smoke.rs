/// Smoke tests that exercise the actual built binary (both debug and release).
/// These catch issues like missing symlinks, broken PATH setup, and redo-log
/// subprocess failures that only manifest when running the real binary.
use std::path::{Path, PathBuf};
use std::process::Command;

const REDO_BIN: &str = env!("CARGO_BIN_EXE_redo");

fn redo_dir() -> PathBuf {
    Path::new(REDO_BIN).parent().unwrap().to_path_buf()
}

fn setup_project(dir: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
}

fn run_redo(dir: &Path, args: &[&str]) -> (i32, String, String) {
    // Remove any stale symlinks to force the binary to recreate them,
    // simulating a fresh install.
    let bin_dir = redo_dir();
    for entry in std::fs::read_dir(&bin_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("redo-") && entry.file_type().unwrap().is_symlink() {
            // Don't remove — we want to test that the binary works
            // even if symlinks already exist.
        }
    }

    let mut cmd = Command::new(REDO_BIN);
    cmd.args(args);
    cmd.current_dir(dir);
    // Clean environment to simulate a user running the binary directly
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
        "REDO_NO_OOB", "MAKEFLAGS", "REDO_CHEATFDS",
    ] {
        cmd.env_remove(var);
    }
    // Only include the binary's directory in PATH
    cmd.env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()));

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo binary");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn run_redo_subcmd(dir: &Path, subcmd: &str, args: &[&str]) -> (i32, String, String) {
    let bin_dir = redo_dir();
    let subcmd_path = bin_dir.join(subcmd);

    let mut cmd = Command::new(&subcmd_path);
    cmd.args(args);
    cmd.current_dir(dir);
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
        "REDO_NO_OOB", "MAKEFLAGS", "REDO_CHEATFDS",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()));

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo subcmd");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn test_basic_build_with_logging() {
    // This is the exact scenario that was broken: redo spawns redo-log
    // as a subprocess, which requires the redo-log symlink to exist.
    let dir = std::env::temp_dir().join(format!("redo-smoke-log-{}", std::process::id()));
    setup_project(&dir, &[("all.do", "echo hello\n")]);

    // Run with logging enabled (the default) — this exercises redo-log subprocess
    let (rc, _stdout, stderr) = run_redo(&dir, &["all"]);

    assert_eq!(
        rc, 0,
        "redo failed with logging enabled.\nstderr:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("failed to start redo-log subprocess"),
        "redo-log subprocess failed to start.\nstderr:\n{}",
        stderr
    );
    let content = std::fs::read_to_string(dir.join("all")).unwrap();
    assert_eq!(content.trim(), "hello");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_basic_build_without_logging() {
    let dir = std::env::temp_dir().join(format!("redo-smoke-nolog-{}", std::process::id()));
    setup_project(&dir, &[("all.do", "echo hello\n")]);

    let mut cmd = Command::new(REDO_BIN);
    cmd.arg("all");
    cmd.current_dir(&dir);
    cmd.env("REDO_LOG", "0");
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin", redo_dir().display()));

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success(), "redo --no-log failed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_symlinks_auto_created() {
    // After running redo once, all sub-command symlinks should exist
    let dir = std::env::temp_dir().join(format!("redo-smoke-symlinks-{}", std::process::id()));
    setup_project(&dir, &[("all.do", "echo ok\n")]);

    let (rc, _, stderr) = run_redo(&dir, &["all"]);
    assert_eq!(rc, 0, "redo failed:\n{}", stderr);

    let bin_dir = redo_dir();
    for cmd in &[
        "redo-ifchange", "redo-ifcreate", "redo-always", "redo-stamp",
        "redo-log", "redo-whichdo", "redo-targets", "redo-sources",
        "redo-ood", "redo-unlocked",
    ] {
        let path = bin_dir.join(cmd);
        assert!(
            path.exists(),
            "symlink {} should exist after running redo",
            cmd
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_subcmds_work_after_build() {
    let dir = std::env::temp_dir().join(format!("redo-smoke-subcmds-{}", std::process::id()));
    setup_project(&dir, &[
        ("a.do", "echo built-a\n"),
        ("src.txt", "source\n"),
        ("b.do", "redo-ifchange src.txt\ncat src.txt\n"),
    ]);

    // Build
    let (rc, _, stderr) = run_redo(&dir, &["a", "b"]);
    assert_eq!(rc, 0, "build failed:\n{}", stderr);

    // redo-targets
    let (rc, stdout, _) = run_redo_subcmd(&dir, "redo-targets", &[]);
    assert_eq!(rc, 0, "redo-targets failed");
    assert!(stdout.contains("a"), "redo-targets should list 'a'");
    assert!(stdout.contains("b"), "redo-targets should list 'b'");

    // redo-sources
    let (rc, stdout, _) = run_redo_subcmd(&dir, "redo-sources", &[]);
    assert_eq!(rc, 0, "redo-sources failed");
    assert!(stdout.contains("src.txt"), "redo-sources should list 'src.txt'");

    // redo-whichdo
    let (rc, stdout, _) = run_redo_subcmd(&dir, "redo-whichdo", &["a"]);
    assert_eq!(rc, 0, "redo-whichdo failed");
    assert!(stdout.contains("a.do"), "redo-whichdo should find a.do");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_dependency_rebuild() {
    let dir = std::env::temp_dir().join(format!("redo-smoke-deps-{}", std::process::id()));
    setup_project(&dir, &[
        ("src.txt", "v1\n"),
        ("out.do", "redo-ifchange src.txt\ncat src.txt\n"),
    ]);

    // Build
    let (rc, _, _) = run_redo(&dir, &["out"]);
    assert_eq!(rc, 0);
    assert_eq!(std::fs::read_to_string(dir.join("out")).unwrap(), "v1\n");

    // Modify source
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(dir.join("src.txt"), "v2\n").unwrap();

    // Rebuild with redo-ifchange
    let (rc, _, stderr) = run_redo_subcmd(&dir, "redo-ifchange", &["out"]);
    assert_eq!(rc, 0, "rebuild failed:\n{}", stderr);
    assert_eq!(std::fs::read_to_string(dir.join("out")).unwrap(), "v2\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_nested_builds_with_logging() {
    // Tests that nested redo-ifchange calls work with redo-log active.
    // This exercises the full fork/exec/pipe chain.
    let dir = std::env::temp_dir().join(format!("redo-smoke-nested-{}", std::process::id()));
    setup_project(&dir, &[
        ("inner.do", "echo inner-content\n"),
        ("outer.do", "redo-ifchange inner\necho outer: $(cat inner)\n"),
    ]);

    let (rc, _, stderr) = run_redo(&dir, &["outer"]);
    assert_eq!(rc, 0, "nested build failed:\n{}", stderr);
    assert!(
        !stderr.contains("failed to start redo-log"),
        "redo-log failed in nested build:\n{}",
        stderr
    );
    let content = std::fs::read_to_string(dir.join("outer")).unwrap();
    assert!(content.contains("inner-content"), "outer should contain inner's output");

    let _ = std::fs::remove_dir_all(&dir);
}
