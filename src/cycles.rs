// cycles.rs - Cyclic dependency detection
//
// Based on redo/cycles.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use std::collections::HashSet;

#[derive(Debug)]
pub struct CyclicDependencyError;

impl std::fmt::Display for CyclicDependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "cyclic dependency detected")
    }
}

impl std::error::Error for CyclicDependencyError {}

fn get_cycles() -> HashSet<String> {
    std::env::var("REDO_CYCLES")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn add(fid: i64) {
    let mut items = get_cycles();
    items.insert(fid.to_string());
    let joined: Vec<String> = items.into_iter().collect();
    std::env::set_var("REDO_CYCLES", joined.join(":"));
}

pub fn check(fid: i64) -> Result<(), CyclicDependencyError> {
    if get_cycles().contains(&fid.to_string()) {
        Err(CyclicDependencyError)
    } else {
        Ok(())
    }
}
