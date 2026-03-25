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

/// Result of running redo on a project.
#[derive(Debug)]
struct RedoResult {
    exit_code: i32,
    /// Map of target name → file contents (None if file doesn't exist).
    files: BTreeMap<String, Option<String>>,
}

/// Set up a project directory with the given files.
fn write_project(dir: &Path, files: &BTreeMap<String, String>) {
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
}

/// Run a redo implementation on a project directory and collect results.
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
    // Remove any inherited redo state
    cmd.env_remove("REDO");
    cmd.env_remove("REDO_BASE");
    cmd.env_remove("REDO_STARTDIR");
    cmd.env_remove("REDO_TARGET");
    cmd.env_remove("REDO_PWD");
    cmd.env_remove("REDO_RUNID");
    cmd.env_remove("REDO_DEPTH");
    cmd.env_remove("REDO_CYCLES");
    cmd.env_remove("REDO_UNLOCKED");
    // Ensure our redo bin is first in PATH for sub-invocations
    let path = format!(
        "{}:/usr/bin:/bin:/usr/sbin:/sbin",
        redo_path_prefix
    );
    cmd.env("PATH", &path);
    // Timeout
    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo");

    let exit_code = output.status.code().unwrap_or(-1);

    let mut files = BTreeMap::new();
    for name in collect_files {
        let path = project_dir.join(name);
        let content = std::fs::read_to_string(&path).ok();
        files.insert(name.to_string(), content);
    }

    RedoResult { exit_code, files }
}

/// Run a redo sub-command (redo-ifchange, redo-targets, etc.) and capture stdout.
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
    cmd.env_remove("REDO");
    cmd.env_remove("REDO_BASE");
    cmd.env_remove("REDO_STARTDIR");
    cmd.env_remove("REDO_TARGET");
    cmd.env_remove("REDO_PWD");
    cmd.env_remove("REDO_RUNID");
    cmd.env_remove("REDO_DEPTH");
    cmd.env_remove("REDO_CYCLES");
    cmd.env_remove("REDO_UNLOCKED");
    let path = format!(
        "{}:/usr/bin:/bin:/usr/sbin:/sbin",
        redo_path_prefix
    );
    cmd.env("PATH", &path);

    let output = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("failed to run redo cmd");

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (exit_code, stdout)
}

/// Get the directory for the Rust redo symlinks.
fn rust_redo_dir() -> PathBuf {
    // The symlinks live alongside the binary
    Path::new(RUST_REDO_BIN).parent().unwrap().to_path_buf()
}

/// Create a temp directory for a test, returning (python_dir, rust_dir).
fn create_test_dirs(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("redo-equiv-{}-{}", name, std::process::id()));
    let py_dir = base.join("python");
    let rs_dir = base.join("rust");
    std::fs::create_dir_all(&py_dir).unwrap();
    std::fs::create_dir_all(&rs_dir).unwrap();
    (py_dir, rs_dir)
}

fn cleanup_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// Ensure the Rust redo symlinks exist in the release binary directory.
fn ensure_rust_symlinks() {
    let dir = rust_redo_dir();
    let redo_bin = dir.join("redo");
    for cmd in &[
        "redo-ifchange",
        "redo-ifcreate",
        "redo-always",
        "redo-stamp",
        "redo-log",
        "redo-whichdo",
        "redo-targets",
        "redo-sources",
        "redo-ood",
        "redo-unlocked",
    ] {
        let link = dir.join(cmd);
        if !link.exists() {
            std::os::unix::fs::symlink(&redo_bin, &link).unwrap();
        }
    }
}

/// Ensure the Python redo wrappers exist.
fn ensure_python_wrappers() {
    let dir = Path::new(PYTHON_REDO_DIR);
    if dir.join("redo").exists() {
        return;
    }
    std::fs::create_dir_all(dir).unwrap();
    let commands: &[(&str, &str)] = &[
        ("redo", "cmd_redo"),
        ("redo-ifchange", "cmd_ifchange"),
        ("redo-ifcreate", "cmd_ifcreate"),
        ("redo-always", "cmd_always"),
        ("redo-stamp", "cmd_stamp"),
        ("redo-log", "cmd_log"),
        ("redo-whichdo", "cmd_whichdo"),
        ("redo-targets", "cmd_targets"),
        ("redo-sources", "cmd_sources"),
        ("redo-ood", "cmd_ood"),
        ("redo-unlocked", "cmd_unlocked"),
    ];
    // Create version shim
    let version_dir = Path::new("/tmp/apenwarr-redo/redo/version");
    std::fs::create_dir_all(version_dir).unwrap();
    std::fs::write(
        version_dir.join("_version.py"),
        "COMMIT = 'test'\nTAG = 'test-0.0'\nDATE = '2024-01-01'\n",
    )
    .unwrap();
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

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// Generate a safe filename component (alphanumeric, no special chars).
fn safe_name_gen() -> impl Generator<String> {
    generators::from_regex(r"[a-z][a-z0-9]{0,7}").fullmatch(true)
}

/// Generate a file extension (or empty).
fn ext_gen() -> impl Generator<String> {
    hegel::one_of!(
        generators::just("".to_string()),
        generators::from_regex(r"\.[a-z]{1,3}").fullmatch(true)
    )
}

/// Generate simple .do file content that produces deterministic output.
/// `deps` is the list of targets this .do file depends on.
/// `source_deps` is the list of source files to depend on.
fn do_content_gen(
    target_name: &str,
    deps: &[String],
    source_deps: &[String],
    use_stamp: bool,
    use_always: bool,
) -> String {
    let mut lines = Vec::new();
    // Depend on source files and other targets
    let mut all_deps: Vec<String> = Vec::new();
    all_deps.extend(deps.iter().cloned());
    all_deps.extend(source_deps.iter().cloned());
    if !all_deps.is_empty() {
        lines.push(format!("redo-ifchange {}", all_deps.join(" ")));
    }
    if use_always {
        lines.push("redo-always".to_string());
    }
    // Produce output: concat source deps content + own name
    for src in source_deps {
        lines.push(format!("cat {}", src));
    }
    lines.push(format!("echo {}", target_name));
    if use_stamp {
        // Pipe output through redo-stamp
        let content_lines = lines.join("\n");
        return format!(
            "(\n{}\n) | redo-stamp\n(\n{}\n) >$3\n",
            content_lines, content_lines
        );
    }
    lines.join("\n")
}

/// A generated project description.
#[derive(Debug, Clone)]
struct Project {
    /// Source files: name -> content
    sources: BTreeMap<String, String>,
    /// Target .do files: target_name -> .do file content
    targets: BTreeMap<String, String>,
    /// All target names (for collecting results)
    target_names: Vec<String>,
}

impl Project {
    fn to_files(&self) -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        for (name, content) in &self.sources {
            files.insert(name.clone(), content.clone());
        }
        for (name, content) in &self.targets {
            files.insert(format!("{}.do", name), content.clone());
        }
        files
    }
}

/// Generate a random project with a DAG of dependencies.
#[hegel::composite]
fn gen_project(tc: hegel::TestCase) -> Project {
    let n_sources = tc.draw(generators::integers::<usize>().min_value(0).max_value(4));
    let n_targets = tc.draw(generators::integers::<usize>().min_value(1).max_value(6));

    let mut sources = BTreeMap::new();
    let mut source_names = Vec::new();
    for i in 0..n_sources {
        let name = format!("src{}", i);
        let content = tc.draw(generators::from_regex(r"[a-z ]{1,20}").fullmatch(true));
        sources.insert(name.clone(), format!("{}\n", content));
        source_names.push(name);
    }

    let mut target_names = Vec::new();
    for i in 0..n_targets {
        let name_base = tc.draw(safe_name_gen());
        let ext = tc.draw(ext_gen());
        let name = format!("{}{}", name_base, ext);
        // Avoid duplicates
        if target_names.contains(&name) || source_names.contains(&name) {
            continue;
        }
        target_names.push(name);
    }
    tc.assume(!target_names.is_empty());

    // Generate dependency edges (only forward to avoid cycles)
    let mut targets = BTreeMap::new();
    for (idx, name) in target_names.iter().enumerate() {
        // Can only depend on targets with lower indices (DAG)
        let mut deps = Vec::new();
        for prev_idx in 0..idx {
            if tc.draw(generators::booleans()) {
                deps.push(target_names[prev_idx].clone());
            }
        }
        // Pick some source deps
        let mut src_deps = Vec::new();
        for src in &source_names {
            if tc.draw(generators::booleans()) {
                src_deps.push(src.clone());
            }
        }
        let use_stamp = tc.draw(generators::integers::<u8>().max_value(4)) == 0;
        let use_always = tc.draw(generators::integers::<u8>().max_value(6)) == 0;
        let content = do_content_gen(name, &deps, &src_deps, use_stamp, use_always);
        targets.insert(name.clone(), content);
    }

    Project {
        sources,
        targets,
        target_names,
    }
}

/// Generate a sequence of operations (build, modify source, rebuild).
#[derive(Debug, Clone)]
enum Op {
    Build(Vec<String>),
    ModifySource(String, String),
    TouchSource(String),
    DeleteTarget(String),
}

#[hegel::composite]
fn gen_ops(tc: hegel::TestCase, project: Project) -> Vec<Op> {
    let n_ops = tc.draw(generators::integers::<usize>().min_value(1).max_value(5));
    let mut ops = Vec::new();

    // Always start with a build of all targets
    ops.push(Op::Build(project.target_names.clone()));

    for _ in 1..n_ops {
        let op_type = tc.draw(generators::integers::<u8>().max_value(3));
        match op_type {
            0 => {
                // Rebuild subset of targets
                let mut subset = Vec::new();
                for t in &project.target_names {
                    if tc.draw(generators::booleans()) {
                        subset.push(t.clone());
                    }
                }
                if subset.is_empty() {
                    subset = project.target_names.clone();
                }
                ops.push(Op::Build(subset));
            }
            1 if !project.sources.is_empty() => {
                // Modify a source file
                let src_names: Vec<&String> = project.sources.keys().collect();
                let idx = tc.draw(
                    generators::integers::<usize>().max_value(src_names.len() - 1),
                );
                let new_content =
                    tc.draw(generators::from_regex(r"[a-z ]{1,20}").fullmatch(true));
                ops.push(Op::ModifySource(
                    src_names[idx].clone(),
                    format!("{}\n", new_content),
                ));
            }
            2 if !project.sources.is_empty() => {
                // Touch a source (change mtime but not content)
                let src_names: Vec<&String> = project.sources.keys().collect();
                let idx = tc.draw(
                    generators::integers::<usize>().max_value(src_names.len() - 1),
                );
                ops.push(Op::TouchSource(src_names[idx].clone()));
            }
            _ => {
                // Rebuild all
                ops.push(Op::Build(project.target_names.clone()));
            }
        }
    }
    ops
}

/// Run a sequence of operations on a project dir.
fn run_ops(redo_prefix: &str, dir: &Path, ops: &[Op]) -> Vec<(String, RedoResult)> {
    let mut results = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Build(targets) => {
                let target_refs: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                // Collect all files that might be generated
                let collect: Vec<&str> = targets.iter().map(|s| s.as_str()).collect();
                let result = run_redo(redo_prefix, dir, &target_refs, &collect);
                results.push((format!("build-{}", i), result));
            }
            Op::ModifySource(name, content) => {
                std::fs::write(dir.join(name), content).unwrap();
                // Small sleep to ensure mtime changes
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Op::TouchSource(name) => {
                let path = dir.join(name);
                let content = std::fs::read_to_string(&path).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(10));
                std::fs::write(&path, &content).unwrap();
            }
            Op::DeleteTarget(name) => {
                let _ = std::fs::remove_file(dir.join(name));
            }
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Both implementations build the same targets and produce the same output files.
#[hegel::test(test_cases = 50, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_build_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    tc.note(&format!("Project: {:?}", project));

    let (py_dir, rs_dir) = create_test_dirs("build");
    let files = project.to_files();
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();
    let collect_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    let py_result = run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &collect_refs);
    let rs_result = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &target_refs,
        &collect_refs,
    );

    assert_eq!(
        py_result.exit_code, rs_result.exit_code,
        "Exit codes differ: python={}, rust={}\nProject: {:?}",
        py_result.exit_code, rs_result.exit_code, project
    );

    for name in &project.target_names {
        assert_eq!(
            py_result.files.get(name),
            rs_result.files.get(name),
            "File contents differ for {:?}\npython: {:?}\nrust: {:?}",
            name,
            py_result.files.get(name),
            rs_result.files.get(name),
        );
    }

    cleanup_dir(&py_dir);
}

/// After an initial build, both report the same targets/sources.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_targets_sources_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("tgt-src");
    let files = project.to_files();
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();
    let collect_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    // Build first
    run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &collect_refs);
    run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &target_refs,
        &collect_refs,
    );

    // Compare redo-targets
    let (py_rc, py_targets) = run_redo_cmd(PYTHON_REDO_DIR, "redo-targets", &py_dir, &[]);
    let (rs_rc, rs_targets) =
        run_redo_cmd(&rust_redo_dir().to_string_lossy(), "redo-targets", &rs_dir, &[]);

    assert_eq!(py_rc, rs_rc, "redo-targets exit code differs");
    let py_set: BTreeSet<&str> = py_targets.lines().collect();
    let rs_set: BTreeSet<&str> = rs_targets.lines().collect();
    assert_eq!(
        py_set, rs_set,
        "redo-targets output differs\npython: {:?}\nrust: {:?}",
        py_set, rs_set
    );

    // Compare redo-sources
    let (py_rc, py_sources) = run_redo_cmd(PYTHON_REDO_DIR, "redo-sources", &py_dir, &[]);
    let (rs_rc, rs_sources) =
        run_redo_cmd(&rust_redo_dir().to_string_lossy(), "redo-sources", &rs_dir, &[]);

    assert_eq!(py_rc, rs_rc, "redo-sources exit code differs");
    let py_set: BTreeSet<&str> = py_sources.lines().collect();
    let rs_set: BTreeSet<&str> = rs_sources.lines().collect();
    assert_eq!(
        py_set, rs_set,
        "redo-sources output differs\npython: {:?}\nrust: {:?}",
        py_set, rs_set
    );

    cleanup_dir(&py_dir);
}

/// After build + modify source + rebuild, both produce the same output.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_rebuild_after_modify(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    tc.assume(!project.sources.is_empty());

    let ops = tc.draw(gen_ops(project.clone()));
    tc.note(&format!("Ops: {:?}", ops));

    let (py_dir, rs_dir) = create_test_dirs("rebuild");
    let files = project.to_files();
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let py_results = run_ops(PYTHON_REDO_DIR, &py_dir, &ops);
    let rs_results = run_ops(&rust_redo_dir().to_string_lossy(), &rs_dir, &ops);

    assert_eq!(
        py_results.len(),
        rs_results.len(),
        "Different number of build results"
    );

    for ((py_label, py_res), (rs_label, rs_res)) in py_results.iter().zip(rs_results.iter()) {
        assert_eq!(py_label, rs_label);
        assert_eq!(
            py_res.exit_code, rs_res.exit_code,
            "Exit code differs at {}: python={}, rust={}",
            py_label, py_res.exit_code, rs_res.exit_code
        );
        for (name, py_content) in &py_res.files {
            let rs_content = rs_res.files.get(name);
            assert_eq!(
                Some(py_content),
                rs_content,
                "File {:?} differs at {}\npython: {:?}\nrust: {:?}",
                name,
                py_label,
                py_content,
                rs_content,
            );
        }
    }

    cleanup_dir(&py_dir);
}

/// redo-whichdo returns the same .do file search list for both implementations.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_whichdo_equivalence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("whichdo");
    let files = project.to_files();
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    for name in &project.target_names {
        let (py_rc, py_out) =
            run_redo_cmd(PYTHON_REDO_DIR, "redo-whichdo", &py_dir, &[name.as_str()]);
        let (rs_rc, rs_out) = run_redo_cmd(
            &rust_redo_dir().to_string_lossy(),
            "redo-whichdo",
            &rs_dir,
            &[name.as_str()],
        );

        assert_eq!(
            py_rc, rs_rc,
            "redo-whichdo exit code differs for {:?}: python={}, rust={}",
            name, py_rc, rs_rc
        );
        assert_eq!(
            py_out, rs_out,
            "redo-whichdo output differs for {:?}\npython:\n{}\nrust:\n{}",
            name, py_out, rs_out
        );
    }

    cleanup_dir(&py_dir);
}

/// When a target has no .do file, both implementations fail with the same exit code.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow])]
fn test_missing_do_file(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let target_name = tc.draw(safe_name_gen());
    // Don't create a .do file for this target

    let (py_dir, rs_dir) = create_test_dirs("missing");

    let py_result = run_redo(PYTHON_REDO_DIR, &py_dir, &[&target_name], &[&target_name]);
    let rs_result = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &[&target_name],
        &[&target_name],
    );

    // Both should fail
    assert_ne!(py_result.exit_code, 0, "Python should fail for missing .do");
    assert_ne!(rs_result.exit_code, 0, "Rust should fail for missing .do");

    cleanup_dir(&py_dir);
}

/// Building the same targets twice (idempotence): output files unchanged.
#[hegel::test(test_cases = 30, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_build_idempotence(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let project = tc.draw(gen_project());
    let (py_dir, rs_dir) = create_test_dirs("idemp");
    let files = project.to_files();
    write_project(&py_dir, &files);
    write_project(&rs_dir, &files);

    let target_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();
    let collect_refs: Vec<&str> = project.target_names.iter().map(|s| s.as_str()).collect();

    // Build twice with Python
    run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &collect_refs);
    let py_result2 = run_redo(PYTHON_REDO_DIR, &py_dir, &target_refs, &collect_refs);

    // Build twice with Rust
    run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &target_refs,
        &collect_refs,
    );
    let rs_result2 = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &target_refs,
        &collect_refs,
    );

    // Second build of each should produce the same results
    assert_eq!(py_result2.exit_code, rs_result2.exit_code);
    for name in &project.target_names {
        assert_eq!(
            py_result2.files.get(name),
            rs_result2.files.get(name),
            "Second build differs for {:?}",
            name,
        );
    }

    cleanup_dir(&py_dir);
}

/// Default.do files with extensions work the same way.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_default_do_matching(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    // Create a project with default.do and optionally default.ext.do
    let ext = tc.draw(generators::from_regex(r"\.[a-z]{1,3}").fullmatch(true));
    let base_name = tc.draw(safe_name_gen());
    let target = format!("{}{}", base_name, ext);

    let has_specific = tc.draw(generators::booleans());

    let mut project_files = BTreeMap::new();
    project_files.insert(
        "default.do".to_string(),
        format!("echo default $1 $2 ${{1#$2}}\n"),
    );
    if has_specific {
        project_files.insert(
            format!("default{}.do", ext),
            format!("echo specific-{} $1 $2 ${{1#$2}}\n", ext),
        );
    }

    let (py_dir, rs_dir) = create_test_dirs("default-do");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    let py_result = run_redo(PYTHON_REDO_DIR, &py_dir, &[&target], &[&target]);
    let rs_result = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &[&target],
        &[&target],
    );

    assert_eq!(
        py_result.exit_code, rs_result.exit_code,
        "Exit code differs for target {:?}: python={}, rust={}",
        target, py_result.exit_code, rs_result.exit_code
    );
    assert_eq!(
        py_result.files.get(&target),
        rs_result.files.get(&target),
        "Output differs for {:?} (has_specific={})\npython: {:?}\nrust: {:?}",
        target,
        has_specific,
        py_result.files.get(&target),
        rs_result.files.get(&target),
    );

    cleanup_dir(&py_dir);
}

/// A failing .do script produces the same error behavior.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow])]
fn test_failing_do_script(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let target_name = tc.draw(safe_name_gen());
    let exit_code = tc.draw(generators::integers::<u8>().min_value(1).max_value(125));

    let mut project_files = BTreeMap::new();
    project_files.insert(
        format!("{}.do", target_name),
        format!("exit {}\n", exit_code),
    );

    let (py_dir, rs_dir) = create_test_dirs("fail");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    let py_result = run_redo(PYTHON_REDO_DIR, &py_dir, &[&target_name], &[&target_name]);
    let rs_result = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &[&target_name],
        &[&target_name],
    );

    // Both should fail (exit non-zero)
    assert_ne!(py_result.exit_code, 0, "Python should fail");
    assert_ne!(rs_result.exit_code, 0, "Rust should fail");

    // Target should not be created by either
    assert_eq!(
        py_result.files.get(&target_name),
        rs_result.files.get(&target_name),
        "Target file existence differs"
    );

    cleanup_dir(&py_dir);
}

/// redo-stamp: checksum-based change detection agrees.
#[hegel::test(test_cases = 20, suppress_health_check = [HealthCheck::TooSlow, HealthCheck::FilterTooMuch])]
fn test_stamp_based_deps(tc: hegel::TestCase) {
    ensure_rust_symlinks();
    ensure_python_wrappers();

    let content1 = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));
    let content2 = tc.draw(generators::from_regex(r"[a-z]{1,10}").fullmatch(true));

    let mut project_files = BTreeMap::new();
    project_files.insert("src.txt".to_string(), format!("{}\n", content1));
    // stamped target: output is hash-based, only changes when content changes
    project_files.insert(
        "stamped.do".to_string(),
        "redo-ifchange src.txt\ncat src.txt | redo-stamp\ncat src.txt >$3\n".to_string(),
    );
    // consumer depends on stamped
    project_files.insert(
        "consumer.do".to_string(),
        "redo-ifchange stamped\necho consumed: $(cat stamped)\n".to_string(),
    );

    let (py_dir, rs_dir) = create_test_dirs("stamp");
    write_project(&py_dir, &project_files);
    write_project(&rs_dir, &project_files);

    // Build once
    let py_r1 = run_redo(
        PYTHON_REDO_DIR,
        &py_dir,
        &["consumer"],
        &["consumer", "stamped"],
    );
    let rs_r1 = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &["consumer"],
        &["consumer", "stamped"],
    );
    assert_eq!(py_r1.exit_code, rs_r1.exit_code);
    assert_eq!(py_r1.files, rs_r1.files, "First build differs");

    // Modify source with different content
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(py_dir.join("src.txt"), format!("{}\n", content2)).unwrap();
    std::fs::write(rs_dir.join("src.txt"), format!("{}\n", content2)).unwrap();

    // Rebuild
    let py_r2 = run_redo(
        PYTHON_REDO_DIR,
        &py_dir,
        &["consumer"],
        &["consumer", "stamped"],
    );
    let rs_r2 = run_redo(
        &rust_redo_dir().to_string_lossy(),
        &rs_dir,
        &["consumer"],
        &["consumer", "stamped"],
    );
    assert_eq!(py_r2.exit_code, rs_r2.exit_code);
    assert_eq!(
        py_r2.files, rs_r2.files,
        "Rebuild after modify differs\npython: {:?}\nrust: {:?}",
        py_r2.files, rs_r2.files
    );

    cleanup_dir(&py_dir);
}
