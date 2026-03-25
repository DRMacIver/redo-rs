/// Property-based equivalence tests: run both Python redo (apenwarr/redo) and
/// Rust redo on randomly generated projects and assert identical behavior.
use hegel::generators::{self, Generator};
use hegel::HealthCheck;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const RUST_REDO_BIN: &str = env!("CARGO_BIN_EXE_redo");
const PYTHON_REDO_DIR: &str = "/tmp/python-redo-bin";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RedoResult {
    exit_code: i32,
    files: BTreeMap<String, Option<String>>,
}

fn write_project(dir: &Path, files: &BTreeMap<String, String>) {
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
}

fn run_redo(
    redo_path_prefix: &str,
    project_dir: &Path,
    targets: &[&str],
    collect_files: &[&str],
) -> RedoResult {
    let mut cmd = Command::new(format!("{}/redo", redo_path_prefix));
    cmd.args(targets);
    cmd.current_dir(project_dir);
    cmd.env("REDO_LOG", "0");
    cmd.env("REDO_NO_OOB", "1");
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", redo_path_prefix));
    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo");

    let mut files = BTreeMap::new();
    for name in collect_files {
        files.insert(name.to_string(), std::fs::read_to_string(project_dir.join(name)).ok());
    }
    RedoResult {
        exit_code: output.status.code().unwrap_or(-1),
        files,
    }
}

fn run_redo_ifchange(
    redo_path_prefix: &str,
    project_dir: &Path,
    targets: &[&str],
    collect_files: &[&str],
) -> RedoResult {
    let mut cmd = Command::new(format!("{}/redo-ifchange", redo_path_prefix));
    cmd.args(targets);
    cmd.current_dir(project_dir);
    cmd.env("REDO_LOG", "0");
    cmd.env("REDO_NO_OOB", "1");
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", redo_path_prefix));
    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo-ifchange");

    let mut files = BTreeMap::new();
    for name in collect_files {
        files.insert(name.to_string(), std::fs::read_to_string(project_dir.join(name)).ok());
    }
    RedoResult {
        exit_code: output.status.code().unwrap_or(-1),
        files,
    }
}

fn run_redo_cmd(
    redo_path_prefix: &str,
    cmd_name: &str,
    project_dir: &Path,
    args: &[&str],
) -> (i32, String) {
    let mut cmd = Command::new(format!("{}/{}", redo_path_prefix, cmd_name));
    cmd.args(args);
    cmd.current_dir(project_dir);
    cmd.env("REDO_LOG", "0");
    cmd.env("REDO_NO_OOB", "1");
    for var in &[
        "REDO", "REDO_BASE", "REDO_STARTDIR", "REDO_TARGET", "REDO_PWD",
        "REDO_RUNID", "REDO_DEPTH", "REDO_CYCLES", "REDO_UNLOCKED",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", redo_path_prefix));
    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo cmd");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

fn rust_redo_dir() -> PathBuf {
    Path::new(RUST_REDO_BIN).parent().unwrap().to_path_buf()
}

fn create_test_dirs(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "redo-equiv-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    let py_dir = base.join("python");
    let rs_dir = base.join("rust");
    std::fs::create_dir_all(&py_dir).unwrap();
    std::fs::create_dir_all(&rs_dir).unwrap();
    (py_dir, rs_dir)
}

fn cleanup_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

fn ensure_rust_symlinks() {
    let dir = rust_redo_dir();
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

fn rs_prefix() -> String {
    rust_redo_dir().to_string_lossy().to_string()
}

fn assert_eq_results(label: &str, py: &RedoResult, rs: &RedoResult, context: &str) {
    assert_eq!(
        py.exit_code, rs.exit_code,
        "{}: exit codes differ: python={}, rust={}\n{}",
        label, py.exit_code, rs.exit_code, context
    );
    for (name, py_content) in &py.files {
        let rs_content = rs.files.get(name);
        assert_eq!(
            Some(py_content), rs_content,
            "{}: file {:?} differs\npython: {:?}\nrust: {:?}\n{}",
            label, name, py_content, rs_content, context,
        );
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

fn safe_name_gen() -> impl Generator<String> {
    generators::from_regex(r"[a-z][a-z0-9]{0,5}").fullmatch(true)
}

/// Generate a path component that may include a subdirectory.
fn maybe_subdir_name_gen() -> impl Generator<String> {
    hegel::one_of!(
        // flat name
        generators::from_regex(r"[a-z][a-z0-9]{0,5}").fullmatch(true),
        // one level of subdir
        generators::from_regex(r"[a-z]{1,4}/[a-z][a-z0-9]{0,4}").fullmatch(true)
    )
}

/// Output method for a .do file.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OutputMethod {
    /// Write to stdout (default)
    Stdout,
    /// Write to $3 (temp file)
    DollarThree,
    /// Produce no output (target is deleted)
    NoOutput,
}

/// A generated project with subdirectories, default.do files, and varied output methods.
#[derive(Debug, Clone)]
struct Project {
    /// All files to write: path -> content
    files: BTreeMap<String, String>,
    /// Target names that can be built
    target_names: Vec<String>,
    /// Source file names
    source_names: Vec<String>,
}

/// Generate a project with subdirectories and default.do files.
#[hegel::composite]
fn gen_project(tc: hegel::TestCase) -> Project {
    let n_sources = tc.draw(generators::integers::<usize>().max_value(4));
    let n_targets = tc.draw(generators::integers::<usize>().min_value(1).max_value(8));
    let n_default_dos = tc.draw(generators::integers::<usize>().max_value(2));

    let mut files = BTreeMap::new();
    let mut source_names = Vec::new();
    let mut target_names = Vec::new();
    let mut all_names: BTreeSet<String> = BTreeSet::new();

    // Generate source files (possibly in subdirs)
    for _ in 0..n_sources {
        let name = tc.draw(maybe_subdir_name_gen());
        let src_name = format!("{}.src", name);
        if all_names.contains(&src_name) {
            continue;
        }
        let content = tc.draw(generators::from_regex(r"[a-z ]{1,15}").fullmatch(true));
        files.insert(src_name.clone(), format!("{}\n", content));
        source_names.push(src_name.clone());
        all_names.insert(src_name);
    }

    // Generate targets with explicit .do files
    for i in 0..n_targets {
        let name = tc.draw(maybe_subdir_name_gen());
        if all_names.contains(&name) {
            continue;
        }
        all_names.insert(name.clone());
        target_names.push(name.clone());

        // Choose output method
        let output_method = match tc.draw(generators::integers::<u8>().max_value(4)) {
            0 => OutputMethod::DollarThree,
            1 => OutputMethod::NoOutput,
            _ => OutputMethod::Stdout,
        };

        // Choose deps (only earlier targets to avoid cycles)
        let mut dep_targets: Vec<String> = Vec::new();
        for prev in &target_names[..target_names.len() - 1] {
            if tc.draw(generators::integers::<u8>().max_value(3)) == 0 {
                dep_targets.push(prev.clone());
            }
        }
        let mut dep_sources: Vec<String> = Vec::new();
        for src in &source_names {
            if tc.draw(generators::integers::<u8>().max_value(2)) == 0 {
                dep_sources.push(src.clone());
            }
        }

        let use_stamp = tc.draw(generators::integers::<u8>().max_value(5)) == 0;
        let use_always = tc.draw(generators::integers::<u8>().max_value(7)) == 0;

        let do_content = gen_do_content(
            &name, &dep_targets, &dep_sources,
            output_method, use_stamp, use_always,
        );
        files.insert(format!("{}.do", name), do_content);
    }

    // Generate default.do files at various levels
    for _ in 0..n_default_dos {
        let default_ext = tc.draw(hegel::one_of!(
            generators::just("".to_string()),
            generators::from_regex(r"\.[a-z]{1,3}").fullmatch(true)
        ));
        let default_dir = tc.draw(hegel::one_of!(
            generators::just("".to_string()),
            generators::from_regex(r"[a-z]{1,4}").fullmatch(true)
        ));
        let do_name = if default_dir.is_empty() {
            format!("default{}.do", default_ext)
        } else {
            format!("{}/default{}.do", default_dir, default_ext)
        };
        if files.contains_key(&do_name) {
            continue;
        }
        // default.do scripts use $1, $2, ${1#$2} to show which .do was selected
        files.insert(
            do_name,
            format!("echo default{} $2 ${{1#$2}}\n", default_ext),
        );
    }

    tc.assume(!target_names.is_empty());

    Project {
        files,
        target_names,
        source_names,
    }
}

fn gen_do_content(
    target_name: &str,
    dep_targets: &[String],
    dep_sources: &[String],
    output_method: OutputMethod,
    use_stamp: bool,
    use_always: bool,
) -> String {
    let mut lines = Vec::new();

    let all_deps: Vec<&String> = dep_targets.iter().chain(dep_sources.iter()).collect();
    if !all_deps.is_empty() {
        let dep_strs: Vec<&str> = all_deps.iter().map(|d| d.as_str()).collect();
        lines.push(format!("redo-ifchange {}", dep_strs.join(" ")));
    }
    if use_always {
        lines.push("redo-always".to_string());
    }

    // Build the output content
    let mut output_lines = Vec::new();
    for src in dep_sources {
        output_lines.push(format!("cat {}", src));
    }
    // Include target name so each target has distinct content
    output_lines.push(format!("echo built:{}", target_name));

    match output_method {
        OutputMethod::Stdout => {
            if use_stamp {
                lines.push(format!(
                    "(\n{}\n) | redo-stamp",
                    output_lines.join("\n")
                ));
                lines.push(format!("(\n{}\n) >$3", output_lines.join("\n")));
            } else {
                lines.extend(output_lines);
            }
        }
        OutputMethod::DollarThree => {
            if use_stamp {
                lines.push(format!(
                    "(\n{}\n) | redo-stamp",
                    output_lines.join("\n")
                ));
            }
            lines.push(format!("(\n{}\n) >$3", output_lines.join("\n")));
        }
        OutputMethod::NoOutput => {
            // Produce no output - target file should be deleted
        }
    }

    lines.join("\n") + "\n"
}

/// Sequence of operations for multi-step testing.
#[derive(Debug, Clone)]
enum Op {
    /// Build targets with `redo`
    Redo(Vec<String>),
    /// Build targets with `redo-ifchange`
    RedoIfchange(Vec<String>),
    /// Modify a source file's content
    ModifySource { name: String, content: String },
    /// Touch a source (update mtime, same content)
    TouchSource(String),
    /// Delete a target output file
    DeleteTarget(String),
    /// Create a previously non-existent file
    CreateFile { name: String, content: String },
}

#[hegel::composite]
fn gen_ops(tc: hegel::TestCase, project: Project) -> Vec<Op> {
    let n_ops = tc.draw(generators::integers::<usize>().min_value(2).max_value(8));
    let mut ops = Vec::new();

    // Always start with a full build
    ops.push(Op::Redo(project.target_names.clone()));

    for _ in 1..n_ops {
        let op_type = tc.draw(generators::integers::<u8>().max_value(5));
        match op_type {
            0 => {
                // redo (force rebuild) on a subset
                let mut subset = Vec::new();
                for t in &project.target_names {
                    if tc.draw(generators::booleans()) {
                        subset.push(t.clone());
                    }
                }
                if subset.is_empty() {
                    subset.push(project.target_names[0].clone());
                }
                ops.push(Op::Redo(subset));
            }
            1 => {
                // redo-ifchange on a subset
                let mut subset = Vec::new();
                for t in &project.target_names {
                    if tc.draw(generators::booleans()) {
                        subset.push(t.clone());
                    }
                }
                if subset.is_empty() {
                    subset.push(project.target_names[0].clone());
                }
                ops.push(Op::RedoIfchange(subset));
            }
            2 if !project.source_names.is_empty() => {
                let idx = tc.draw(generators::integers::<usize>()
                    .max_value(project.source_names.len() - 1));
                let new_content = tc.draw(generators::from_regex(r"[a-z ]{1,15}").fullmatch(true));
                ops.push(Op::ModifySource {
                    name: project.source_names[idx].clone(),
                    content: format!("{}\n", new_content),
                });
            }
            3 if !project.source_names.is_empty() => {
                let idx = tc.draw(generators::integers::<usize>()
                    .max_value(project.source_names.len() - 1));
                ops.push(Op::TouchSource(project.source_names[idx].clone()));
            }
            4 if !project.target_names.is_empty() => {
                let idx = tc.draw(generators::integers::<usize>()
                    .max_value(project.target_names.len() - 1));
                ops.push(Op::DeleteTarget(project.target_names[idx].clone()));
            }
            _ => {
                // Full rebuild with redo-ifchange
                ops.push(Op::RedoIfchange(project.target_names.clone()));
            }
        }
    }
    ops
}

fn apply_op(redo_prefix: &str, dir: &Path, op: &Op, collect: &[&str]) -> Option<RedoResult> {
    match op {
        Op::Redo(targets) => {
            let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
            Some(run_redo(redo_prefix, dir, &refs, collect))
        }
        Op::RedoIfchange(targets) => {
            let refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
            Some(run_redo_ifchange(redo_prefix, dir, &refs, collect))
        }
        Op::ModifySource { name, content } => {
            std::fs::write(dir.join(name), content).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            None
        }
        Op::TouchSource(name) => {
            let path = dir.join(name);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::fs::write(&path, &content).unwrap();
            None
        }
        Op::DeleteTarget(name) => {
            let _ = std::fs::remove_file(dir.join(name));
            None
        }
        Op::CreateFile { name, content } => {
            std::fs::write(dir.join(name), content).unwrap();
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Both implementations build targets and produce the same output files.
#[hegel::test(test_cases = 80, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_build_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    tc.note(&format!("Project:\n{:#?}", project));

    let (py_dir, rs_dir) = create_test_dirs("build");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &target_refs);
    let rs = run_redo(&rs_prefix(), &rs_dir, &target_refs, &target_refs);

    assert_eq_results("build", &py, &rs, &format!("{:#?}", project));
    cleanup_dir(&py_dir);
}

/// redo-ifchange: conditional builds produce same results.
#[hegel::test(test_cases = 50, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_ifchange_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("ifchange");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    let py = run_redo_ifchange(PYTHON_REDO_DIR, &py_dir, &target_refs, &target_refs);
    let rs = run_redo_ifchange(&rs_prefix(), &rs_dir, &target_refs, &target_refs);

    assert_eq_results("ifchange", &py, &rs, &format!("{:#?}", project));
    cleanup_dir(&py_dir);
}

/// Multi-step sequences: build, modify, rebuild produce same results.
#[hegel::test(test_cases = 40, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_multi_step_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let ops = tc.draw(gen_ops(project.clone()));
    tc.note(&format!("Project:\n{:#?}\nOps:\n{:#?}", project, ops));

    let (py_dir, rs_dir) = create_test_dirs("multi");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    let collect_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    for (i, op) in ops.iter().enumerate() {
        let py_res = apply_op(PYTHON_REDO_DIR, &py_dir, op, &collect_refs);
        let rs_res = apply_op(&rs_prefix(), &rs_dir, op, &collect_refs);

        if let (Some(py), Some(rs)) = (py_res, rs_res) {
            assert_eq_results(
                &format!("step-{}", i),
                &py,
                &rs,
                &format!("op: {:?}\nproject: {:#?}", op, project),
            );
        }
    }

    cleanup_dir(&py_dir);
}

/// redo-targets and redo-sources agree after building.
#[hegel::test(test_cases = 40, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_targets_sources_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("tgt-src");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();
    run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &[]);
    run_redo(&rs_prefix(), &rs_dir, &target_refs, &[]);

    // redo-targets
    let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-targets", &py_dir, &[]);
    let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-targets", &rs_dir, &[]);
    assert_eq!(py_rc, rs_rc, "redo-targets exit code differs");
    let py_set: BTreeSet<&str> = py_out.lines().collect();
    let rs_set: BTreeSet<&str> = rs_out.lines().collect();
    assert_eq!(py_set, rs_set, "redo-targets differs\npy: {:?}\nrs: {:?}", py_set, rs_set);

    // redo-sources
    let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-sources", &py_dir, &[]);
    let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-sources", &rs_dir, &[]);
    assert_eq!(py_rc, rs_rc, "redo-sources exit code differs");
    let py_set: BTreeSet<&str> = py_out.lines().collect();
    let rs_set: BTreeSet<&str> = rs_out.lines().collect();
    assert_eq!(py_set, rs_set, "redo-sources differs\npy: {:?}\nrs: {:?}", py_set, rs_set);

    cleanup_dir(&py_dir);
}

/// redo-ood (out-of-date) agrees after build + source modification.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_ood_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    tc.assume(!project.source_names.is_empty());
    let (py_dir, rs_dir) = create_test_dirs("ood");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    // Build all
    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();
    run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &[]);
    run_redo(&rs_prefix(), &rs_dir, &target_refs, &[]);

    // Modify a source
    let src = &project.source_names[0];
    let new_content = "modified content\n";
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(py_dir.join(src), new_content).unwrap();
    std::fs::write(rs_dir.join(src), new_content).unwrap();

    // Check redo-ood
    let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-ood", &py_dir, &[]);
    let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-ood", &rs_dir, &[]);
    assert_eq!(py_rc, rs_rc, "redo-ood exit code differs");
    let py_set: BTreeSet<&str> = py_out.lines().collect();
    let rs_set: BTreeSet<&str> = rs_out.lines().collect();
    assert_eq!(py_set, rs_set, "redo-ood differs\npy: {:?}\nrs: {:?}\nsource modified: {}", py_set, rs_set, src);

    cleanup_dir(&py_dir);
}

/// redo-whichdo finds the same .do files for generated targets.
#[hegel::test(test_cases = 40, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_whichdo_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("whichdo");
    write_project(&py_dir, &project.files);
    write_project(&rs_dir, &project.files);

    for name in &project.target_names {
        let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-whichdo", &py_dir, &[name]);
        let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-whichdo", &rs_dir, &[name]);
        assert_eq!(py_rc, rs_rc,
            "redo-whichdo exit code differs for {:?}: py={}, rs={}", name, py_rc, rs_rc);
        assert_eq!(py_out, rs_out,
            "redo-whichdo output differs for {:?}\npy:\n{}\nrs:\n{}", name, py_out, rs_out);
    }

    cleanup_dir(&py_dir);
}

/// default.do matching with multiple extensions works the same.
#[hegel::test(test_cases = 40, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_default_do_extension_matching(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    // Generate a target with multiple extensions like foo.x.y.z
    let base = tc.draw(safe_name_gen());
    let n_exts = tc.draw(generators::integers::<usize>().min_value(1).max_value(3));
    let mut exts = Vec::new();
    for _ in 0..n_exts {
        exts.push(tc.draw(generators::from_regex(r"[a-z]{1,3}").fullmatch(true)));
    }
    let target = format!("{}.{}", base, exts.join("."));

    // Generate some default.do files at various extension levels
    let mut project_files = BTreeMap::new();
    // Always have a root default.do
    project_files.insert("default.do".to_string(), "echo default $1 $2\n".to_string());

    // Maybe add more specific default.ext.do files
    for i in 0..exts.len() {
        if tc.draw(generators::booleans()) {
            let ext = format!(".{}", exts[i..].join("."));
            project_files.insert(
                format!("default{}.do", ext),
                format!("echo default{} $1 $2\n", ext),
            );
        }
    }

    // Maybe add the exact target.do
    if tc.draw(generators::integers::<u8>().max_value(3)) == 0 {
        project_files.insert(
            format!("{}.do", target),
            format!("echo exact $1 $2\n"),
        );
    }

    let (py_dir, rs_dir) = create_test_dirs("ext-match");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    // Compare which .do file is found
    let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-whichdo", &py_dir, &[&target]);
    let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-whichdo", &rs_dir, &[&target]);
    assert_eq!(py_rc, rs_rc, "exit code differs for {:?}", target);
    assert_eq!(py_out, rs_out, "whichdo output differs for {:?}\npy:\n{}rs:\n{}", target, py_out, rs_out);

    // Build and compare output
    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&target], &[&target]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&target], &[&target]);
    assert_eq_results("build", &py, &rs, &format!("target={:?} files={:#?}", target, project_files));

    cleanup_dir(&py_dir);
}

/// Subdirectory default.do resolution: parent dir default.do vs child dir.
#[hegel::test(test_cases = 40, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_subdir_default_do(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let subdir = tc.draw(generators::from_regex(r"[a-z]{1,4}").fullmatch(true));
    let target_base = tc.draw(safe_name_gen());
    let ext = tc.draw(generators::from_regex(r"[a-z]{1,3}").fullmatch(true));
    let target = format!("{}/{}.{}", subdir, target_base, ext);

    let mut project_files = BTreeMap::new();

    // Root default.do
    project_files.insert("default.do".to_string(), "echo root-default $1 $2\n".to_string());

    // Maybe add subdir-specific default.do
    if tc.draw(generators::booleans()) {
        project_files.insert(
            format!("{}/default.do", subdir),
            format!("echo {}-default $1 $2\n", subdir),
        );
    }
    // Maybe add extension-specific default.ext.do
    if tc.draw(generators::booleans()) {
        project_files.insert(
            format!("default.{}.do", ext),
            format!("echo root-default.{} $1 $2\n", ext),
        );
    }
    // Maybe add subdir extension-specific
    if tc.draw(generators::booleans()) {
        project_files.insert(
            format!("{}/default.{}.do", subdir, ext),
            format!("echo {}-default.{} $1 $2\n", subdir, ext),
        );
    }

    // Ensure the subdir exists
    project_files.entry(format!("{}/.gitkeep", subdir))
        .or_insert_with(String::new);

    let (py_dir, rs_dir) = create_test_dirs("subdir");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    // Compare whichdo
    let (py_rc, py_out) = run_redo_cmd(PYTHON_REDO_DIR, "redo-whichdo", &py_dir, &[&target]);
    let (rs_rc, rs_out) = run_redo_cmd(&rs_prefix(), "redo-whichdo", &rs_dir, &[&target]);
    assert_eq!(py_rc, rs_rc, "whichdo exit code for {:?}", target);
    assert_eq!(py_out, rs_out,
        "whichdo output for {:?}\npy:\n{}rs:\n{}\nfiles: {:#?}", target, py_out, rs_out, project_files);

    // Build and compare
    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&target], &[&target]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&target], &[&target]);
    assert_eq_results("build", &py, &rs,
        &format!("target={:?} files={:#?}", target, project_files));

    cleanup_dir(&py_dir);
}

/// Static files (existing before first build) are handled the same.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_static_file_handling(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let name = tc.draw(safe_name_gen());
    let static_content = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));

    let mut project_files = BTreeMap::new();
    // The target file already exists as a "source" (static file)
    project_files.insert(name.clone(), format!("{}\n", static_content));
    // AND there's a .do file for it
    project_files.insert(
        format!("{}.do", name),
        format!("echo built-{}\n", name),
    );

    let (py_dir, rs_dir) = create_test_dirs("static");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    // redo on a pre-existing file: should warn and not rebuild
    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&name], &[&name]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&name], &[&name]);

    // Content should be unchanged (static file not overwritten)
    assert_eq!(
        py.files.get(&name), rs.files.get(&name),
        "Static file handling differs for {:?}\npy: {:?}\nrs: {:?}",
        name, py.files.get(&name), rs.files.get(&name)
    );

    cleanup_dir(&py_dir);
}

/// Failing .do scripts produce the same error behavior.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow])]
fn test_failing_do_script(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let name = tc.draw(safe_name_gen());
    let exit_code = tc.draw(generators::integers::<u8>().min_value(1).max_value(125));

    let mut files = BTreeMap::new();
    files.insert(format!("{}.do", name), format!("exit {}\n", exit_code));

    let (py_dir, rs_dir) = create_test_dirs("fail");
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&name], &[&name]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&name], &[&name]);

    assert_ne!(py.exit_code, 0);
    assert_ne!(rs.exit_code, 0);
    // Both should not create the target
    assert_eq!(py.files.get(&name), rs.files.get(&name));

    cleanup_dir(&py_dir);
}

/// Missing .do files produce same failure.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow])]
fn test_missing_do_file(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let name = tc.draw(safe_name_gen());
    let (py_dir, rs_dir) = create_test_dirs("missing");

    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&name], &[&name]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&name], &[&name]);

    assert_ne!(py.exit_code, 0);
    assert_ne!(rs.exit_code, 0);

    cleanup_dir(&py_dir);
}

/// redo-stamp checksum detection agrees between implementations.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_stamp_change_detection(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let content1 = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));
    let content2 = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));
    let content3 = content1.clone(); // Same as content1 to test no-change case

    let mut files = BTreeMap::new();
    files.insert("src.txt".to_string(), format!("{}\n", content1));
    files.insert("stamped.do".to_string(),
        "redo-ifchange src.txt\ncat src.txt | redo-stamp\ncat src.txt >$3\n".to_string());
    files.insert("consumer.do".to_string(),
        "redo-ifchange stamped\necho consumed $(cat stamped)\n".to_string());

    let (py_dir, rs_dir) = create_test_dirs("stamp2");
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let targets = &["consumer"];
    let collect = &["consumer", "stamped"];

    // Build 1
    let py1 = run_redo(PYTHON_REDO_DIR, &py_dir, targets, collect);
    let rs1 = run_redo(&rs_prefix(), &rs_dir, targets, collect);
    assert_eq_results("build1", &py1, &rs1, "initial build");

    // Modify source to content2
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(py_dir.join("src.txt"), format!("{}\n", content2)).unwrap();
    std::fs::write(rs_dir.join("src.txt"), format!("{}\n", content2)).unwrap();

    // Rebuild (redo-ifchange, not redo)
    let py2 = run_redo_ifchange(PYTHON_REDO_DIR, &py_dir, targets, collect);
    let rs2 = run_redo_ifchange(&rs_prefix(), &rs_dir, targets, collect);
    assert_eq_results("after-modify", &py2, &rs2, &format!("content changed to {:?}", content2));

    // Modify back to content1 (same as original - stamp should detect no change)
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(py_dir.join("src.txt"), format!("{}\n", content3)).unwrap();
    std::fs::write(rs_dir.join("src.txt"), format!("{}\n", content3)).unwrap();

    let py3 = run_redo_ifchange(PYTHON_REDO_DIR, &py_dir, targets, collect);
    let rs3 = run_redo_ifchange(&rs_prefix(), &rs_dir, targets, collect);
    assert_eq_results("after-revert", &py3, &rs3, "content reverted to original");

    cleanup_dir(&py_dir);
}

/// $3 output vs stdout output produce the same results.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow])]
fn test_output_method_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let name = tc.draw(safe_name_gen());
    let content = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));
    let use_dollar3 = tc.draw(generators::booleans());

    let do_content = if use_dollar3 {
        format!("echo {} >$3\n", content)
    } else {
        format!("echo {}\n", content)
    };

    let mut files = BTreeMap::new();
    files.insert(format!("{}.do", name), do_content);

    let (py_dir, rs_dir) = create_test_dirs("output");
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&name], &[&name]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&name], &[&name]);

    assert_eq_results("output", &py, &rs,
        &format!("name={:?} use_dollar3={} content={:?}", name, use_dollar3, content));

    cleanup_dir(&py_dir);
}

/// Empty output .do file: target should be deleted in both.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow])]
fn test_empty_output_deletes_target(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let name = tc.draw(safe_name_gen());

    let mut files = BTreeMap::new();
    // .do that produces no output
    files.insert(format!("{}.do", name), "# no output\n".to_string());

    let (py_dir, rs_dir) = create_test_dirs("empty");
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let py = run_redo(PYTHON_REDO_DIR, &py_dir, &[&name], &[&name]);
    let rs = run_redo(&rs_prefix(), &rs_dir, &[&name], &[&name]);

    assert_eq_results("empty", &py, &rs, &format!("name={:?}", name));
    // Both should have None (file not created)
    assert_eq!(py.files.get(&name), Some(&None), "py should not create file");
    assert_eq!(rs.files.get(&name), Some(&None), "rs should not create file");

    cleanup_dir(&py_dir);
}
