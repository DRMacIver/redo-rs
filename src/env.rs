// env.rs - Environment variable management
//
// Based on redo/env.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::helpers::atoi;
use std::sync::Mutex;

pub static IS_TOPLEVEL: Mutex<bool> = Mutex::new(false);
pub static V: Mutex<Option<Env>> = Mutex::new(None);

fn get(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn get_int(name: &str, default: &str) -> i64 {
    atoi(&get(name, default))
}

fn get_bool(name: &str, default: &str) -> bool {
    !get(name, default).is_empty()
}

#[derive(Debug, Clone)]
pub struct Env {
    pub base: String,
    pub pwd: String,
    pub target: String,
    pub depth: String,
    pub debug: i64,
    pub debug_locks: bool,
    pub debug_pids: bool,
    pub locks_broken: bool,
    pub verbose: i64,
    pub xtrace: i64,
    pub keep_going: bool,
    pub log: i64,
    pub log_inode: String,
    pub color: i64,
    pub pretty: i64,
    pub shuffle: bool,
    pub startdir: String,
    pub runid: Option<i64>,
    pub unlocked: bool,
    pub no_oob: bool,
}

impl Env {
    pub fn from_environment() -> Self {
        let base = std::env::var("REDO_BASE")
            .expect("REDO_BASE not set")
            .trim_end_matches('/')
            .to_string();
        let runid_val = get_int("REDO_RUNID", "");
        Env {
            base,
            pwd: get("REDO_PWD", ""),
            target: get("REDO_TARGET", ""),
            depth: get("REDO_DEPTH", ""),
            debug: atoi(&get("REDO_DEBUG", "")),
            debug_locks: get_bool("REDO_DEBUG_LOCKS", ""),
            debug_pids: get_bool("REDO_DEBUG_PIDS", ""),
            locks_broken: get_bool("REDO_LOCKS_BROKEN", ""),
            verbose: get_int("REDO_VERBOSE", ""),
            xtrace: get_int("REDO_XTRACE", ""),
            keep_going: get_bool("REDO_KEEP_GOING", ""),
            log: get_int("REDO_LOG", "1"),
            log_inode: get("REDO_LOG_INODE", ""),
            color: get_int("REDO_COLOR", ""),
            pretty: get_int("REDO_PRETTY", ""),
            shuffle: get_bool("REDO_SHUFFLE", ""),
            startdir: get("REDO_STARTDIR", ""),
            runid: if runid_val != 0 { Some(runid_val) } else { None },
            unlocked: get_bool("REDO_UNLOCKED", ""),
            no_oob: get_bool("REDO_NO_OOB", ""),
        }
    }
}

/// Read environment variables that must already be set by a parent redo process.
pub fn inherit() {
    if std::env::var("REDO").unwrap_or_default().is_empty() {
        eprintln!(
            "{}: error: must be run from inside a .do",
            std::env::args().next().unwrap_or_default()
        );
        std::process::exit(100);
    }

    *V.lock().unwrap() = Some(Env::from_environment());

    // not inheritable by subprocesses
    std::env::set_var("REDO_UNLOCKED", "");
    std::env::set_var("REDO_NO_OOB", "");
}

/// Start a session for a command that needs no state db.
pub fn init_no_state() {
    {
        let mut tl = IS_TOPLEVEL.lock().unwrap();
        if std::env::var("REDO").unwrap_or_default().is_empty() {
            std::env::set_var("REDO", "NOT_DEFINED");
            *tl = true;
        }
    }
    if std::env::var("REDO_BASE").unwrap_or_default().is_empty() {
        std::env::set_var("REDO_BASE", "NOT_DEFINED");
    }
    inherit();
}

/// Start a session for a command that does need the state db.
pub fn init(targets: &[String]) {
    {
        let mut tl = IS_TOPLEVEL.lock().unwrap();
        if std::env::var("REDO").unwrap_or_default().is_empty() {
            *tl = true;
            let exe = std::env::args().next().unwrap_or_default();
            let abs_exe = std::fs::canonicalize(&exe)
                .unwrap_or_else(|_| std::path::PathBuf::from(&exe));
            let real_exe = std::fs::canonicalize(&abs_exe).unwrap_or(abs_exe.clone());

            let mut dirs: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for p in [&abs_exe, &real_exe] {
                if let Some(dir) = p.parent() {
                    let dir_str = dir.to_string_lossy().to_string();
                    // try lib/redo, ../redo, and dir itself
                    for try_dir in [
                        format!("{}/../lib/redo", dir_str),
                        format!("{}/../redo", dir_str),
                        dir_str.clone(),
                    ] {
                        if let Ok(abs) = std::fs::canonicalize(&try_dir) {
                            let s = abs.to_string_lossy().to_string();
                            if seen.insert(s.clone()) {
                                dirs.push(s);
                            }
                        } else if seen.insert(try_dir.clone()) {
                            dirs.push(try_dir);
                        }
                    }
                }
            }
            // Ensure sibling symlinks exist so execvp("redo-log") etc. work.
            // This is needed because we're a single multi-call binary.
            ensure_sibling_symlinks(&real_exe);

            let path = std::env::var("PATH").unwrap_or_default();
            let new_path = format!("{}:{}", dirs.join(":"), path);
            std::env::set_var("PATH", new_path);
            std::env::set_var(
                "REDO",
                std::fs::canonicalize(&exe)
                    .unwrap_or_else(|_| std::path::PathBuf::from(&exe))
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }

    if std::env::var("REDO_BASE").unwrap_or_default().is_empty() {
        let effective_targets = if targets.is_empty() {
            vec!["all".to_string()]
        } else {
            targets.to_vec()
        };
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut dirs: Vec<String> = effective_targets
            .iter()
            .map(|t| {
                let p = std::path::Path::new(t);
                let abs = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    std::path::PathBuf::from(&cwd).join(p)
                };
                abs.parent()
                    .unwrap_or(std::path::Path::new("/"))
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        dirs.push(cwd.clone());

        // Find common prefix
        let mut base = common_prefix(&dirs);

        // Walk up looking for .redo directory
        let parts: Vec<&str> = base.split('/').collect();
        for i in (1..parts.len()).rev() {
            let newbase = parts[..i].join("/");
            let redo_dir = format!("{}/.redo", newbase);
            if std::path::Path::new(&redo_dir).exists() {
                base = newbase;
                break;
            }
        }

        std::env::set_var("REDO_BASE", &base);
        std::env::set_var("REDO_STARTDIR", &cwd);
    }

    inherit();
}

fn common_prefix(paths: &[String]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let mut prefix = paths[0].clone();
    for p in &paths[1..] {
        while !p.starts_with(&prefix) {
            if let Some(pos) = prefix.rfind('/') {
                prefix.truncate(pos);
            } else {
                prefix.clear();
                break;
            }
        }
    }
    prefix
}

/// Create symlinks for all redo sub-commands next to the main binary.
/// Since we're a single multi-call binary, all commands are the same
/// executable invoked under different names.
fn ensure_sibling_symlinks(exe_path: &std::path::Path) {
    // Only create symlinks if we have a resolved absolute path
    if !exe_path.is_absolute() {
        return;
    }
    let dir = match exe_path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => return,
    };
    let exe_name = exe_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Don't create symlinks if the binary itself is already a symlink
    // target (e.g., we're running as "redo-ifchange" which points to "redo").
    // Only create from the canonical "redo" binary.
    if exe_name != "redo" {
        return;
    }
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
        let link_path = dir.join(cmd);
        if !link_path.exists() {
            let _ = std::os::unix::fs::symlink(exe_path, &link_path);
        }
    }
}

pub fn mark_locks_broken() {
    std::env::set_var("REDO_LOCKS_BROKEN", "1");
    std::env::set_var("REDO_LOG", "0");
    inherit();
}

/// Helper to read the current env. Panics if not initialized.
pub fn with_env<R>(f: impl FnOnce(&Env) -> R) -> R {
    let guard = V.lock().unwrap();
    f(guard.as_ref().expect("env not initialized - call init() first"))
}

/// Helper to modify the current env.
pub fn with_env_mut<R>(f: impl FnOnce(&mut Env) -> R) -> R {
    let mut guard = V.lock().unwrap();
    f(guard.as_mut().expect("env not initialized - call init() first"))
}

pub fn is_toplevel() -> bool {
    *IS_TOPLEVEL.lock().unwrap()
}
