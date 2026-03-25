// paths.rs - .do file discovery and resolution
//
// Based on redo/paths.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, logs, state};

/// (dofile_name, basename, ext) for default .do file candidates
fn default_do_files(filename: &str) -> Vec<(String, String, String)> {
    let parts: Vec<&str> = filename.split('.').collect();
    let mut result = Vec::new();
    for i in 1..=parts.len() {
        let basename = parts[..i].join(".");
        let ext_parts = &parts[i..];
        let ext = if ext_parts.is_empty() {
            String::new()
        } else {
            format!(".{}", ext_parts.join("."))
        };
        result.push((format!("default{}.do", ext), basename, ext));
    }
    result
}

/// Yield the list of (dodir, dofile, basedir, basename, ext) tuples
/// for possible .do files that could build target t.
pub fn possible_do_files(t: &str) -> Vec<(String, String, String, String, String)> {
    let base = env::with_env(|v| v.base.clone());
    let mut result = Vec::new();

    // First: exact target.do
    let path = std::path::Path::new(t);
    let dirname = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Like Python's os.path.join(base, dirname): if dirname is absolute, base is ignored
    let dodir = if std::path::Path::new(&dirname).is_absolute() {
        dirname.clone()
    } else {
        format!("{}/{}", base, dirname).trim_end_matches('/').to_string()
    };
    result.push((
        dodir,
        format!("{}.do", filename),
        String::new(),
        filename.clone(),
        String::new(),
    ));

    // Now: default.*.do in current and parent dirs
    // Like Python's os.path.normpath(os.path.join(base, t))
    let full_t = if std::path::Path::new(t).is_absolute() {
        normalize_path(t)
    } else {
        normalize_path(&format!("{}/{}", base, t))
    };
    let path = std::path::Path::new(&full_t);
    let dirname = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let dirbits: Vec<&str> = dirname.split('/').collect();

    for i in (1..=dirbits.len()).rev() {
        let basedir_path = if i == 0 {
            "/".to_string()
        } else {
            let joined = dirbits[..i].join("/");
            if joined.is_empty() {
                "/".to_string()
            } else {
                joined
            }
        };
        let subdir = dirbits[i..].join("/");

        for (dofile, basename, ext) in default_do_files(&filename) {
            let full_basename = if subdir.is_empty() {
                basename.clone()
            } else {
                format!("{}/{}", subdir, basename)
            };
            result.push((basedir_path.clone(), dofile, subdir.clone(), full_basename, ext));
        }
    }

    result
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let is_absolute = path.starts_with('/');
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() && *parts.last().unwrap() != ".." {
                    parts.pop();
                } else if !is_absolute {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if is_absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Find the first existing .do file for the given state::File.
/// Returns (dodir, dofile, basedir, basename, ext) or (None, None, ...).
pub fn find_do_file(
    f: &state::File,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    for (dodir, dofile, basedir, basename, ext) in possible_do_files(&f.name) {
        let dopath = format!("{}/{}", dodir, dofile);
        logs::debug2(&format!("{}: {}:{} ?", f.name, dodir, dofile));
        if std::path::Path::new(&dopath).exists() {
            f.add_dep("m", &dopath);
            return (
                Some(dodir),
                Some(dofile),
                Some(basedir),
                Some(basename),
                Some(ext),
            );
        } else {
            f.add_dep("c", &dopath);
        }
    }
    (None, None, None, None, None)
}
