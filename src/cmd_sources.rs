// cmd_sources.rs - redo-sources: list known source files
//
// Based on redo/cmd_sources.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, logs, state};

pub fn main() {
    if std::env::args().len() > 1 {
        eprintln!(
            "{}: no arguments expected.",
            std::env::args().next().unwrap_or_default()
        );
        std::process::exit(1);
    }

    state::init(&[]);
    let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
    logs::setup(true, parent_logs != 0, pretty, color);

    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let base = env::with_env(|v| v.base.clone());

    for f in state::files() {
        if f.is_source() {
            println!("{}", state::relpath(&format!("{}/{}", base, f.name), &cwd));
        }
    }
}
