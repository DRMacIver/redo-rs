// cmd_ood.rs - redo-ood: list out-of-date targets
//
// Based on redo/cmd_ood.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{deps, env, logs, state};
use std::collections::HashMap;
use std::sync::Mutex;

static CACHE: Mutex<Option<HashMap<i64, bool>>> = Mutex::new(None);

fn get_cache() -> std::sync::MutexGuard<'static, Option<HashMap<i64, bool>>> {
    let mut guard = CACHE.lock().unwrap();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

fn is_checked(f: &state::File) -> bool {
    let cache = get_cache();
    *cache.as_ref().unwrap().get(&f.id).unwrap_or(&false)
}

fn set_checked(f: &mut state::File) {
    let mut cache = get_cache();
    cache.as_mut().unwrap().insert(f.id, true);
}

fn log_override(_name: &str) {
    // no-op for redo-ood
}

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
    let runid = env::with_env(|v| v.runid.unwrap_or(0));

    for f in state::files() {
        if f.is_target() {
            let dirty = deps::isdirty(
                &f,
                "",
                runid,
                &[],
                Some(&is_checked),
                Some(&set_checked),
                Some(&log_override),
            );
            if let Ok(d) = dirty {
                if d.is_dirty() {
                    println!("{}", state::relpath(&format!("{}/{}", base, f.name), &cwd));
                }
            }
        }
    }
}
