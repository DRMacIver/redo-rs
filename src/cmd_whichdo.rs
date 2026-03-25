// cmd_whichdo.rs - redo-whichdo: find applicable .do files for a target
//
// Based on redo/cmd_whichdo.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, logs, paths};

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 {
        eprintln!(
            "{}: exactly one argument expected.",
            std::env::args().next().unwrap_or_default()
        );
        std::process::exit(1);
    }

    env::init_no_state();
    let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
    logs::setup(true, parent_logs != 0, pretty, color);

    let want = &args[0];
    if want.is_empty() {
        logs::err("cannot build the empty target (\"\").");
        std::process::exit(204);
    }

    let abswant = std::fs::canonicalize(want)
        .unwrap_or_else(|_| {
            let cwd = std::env::current_dir().unwrap();
            cwd.join(want)
        })
        .to_string_lossy()
        .to_string();

    let pdf = paths::possible_do_files(&abswant);
    for (dodir, dofile, _basedir, _basename, _ext) in &pdf {
        let dopath = format!("/{}/{}", dodir, dofile);
        let dopath = dopath.replace("//", "/");
        let cwd = std::env::current_dir().unwrap();
        let relpath = pathdiff::diff_paths(&dopath, &cwd)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(dopath.clone());
        println!("{}", relpath);
        if std::path::Path::new(&dopath).exists() {
            std::process::exit(0);
        }
    }
    std::process::exit(1);
}

mod pathdiff {
    use std::path::{Path, PathBuf};

    pub fn diff_paths(path: &str, base: &Path) -> Option<PathBuf> {
        let path = Path::new(path);
        let mut path_parts: Vec<&std::ffi::OsStr> = path.components().map(|c| c.as_os_str()).collect();
        let mut base_parts: Vec<&std::ffi::OsStr> = base.components().map(|c| c.as_os_str()).collect();

        // Remove common prefix
        while !path_parts.is_empty() && !base_parts.is_empty() && path_parts[0] == base_parts[0] {
            path_parts.remove(0);
            base_parts.remove(0);
        }

        let mut result = PathBuf::new();
        for _ in &base_parts {
            result.push("..");
        }
        for part in &path_parts {
            result.push(part);
        }
        Some(result)
    }
}
