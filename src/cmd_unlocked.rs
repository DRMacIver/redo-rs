// cmd_unlocked.rs - redo-unlocked: internal tool for building dependencies
//
// Based on redo/cmd_unlocked.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, logs};
use nix::sys::wait::WaitStatus;
use nix::unistd::{execvp, fork, ForkResult};

fn spawn_and_wait(argv: &[std::ffi::CString]) -> i32 {
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            let _ = execvp(&argv[0], argv);
            std::process::exit(201);
        }
        Ok(ForkResult::Parent { child }) => {
            match nix::sys::wait::waitpid(child, None) {
                Ok(WaitStatus::Exited(_, code)) => code,
                _ => 201,
            }
        }
        Err(_) => panic!("fork failed"),
    }
}

pub fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!(
            "{}: at least 2 arguments expected.",
            std::env::args().next().unwrap_or_default()
        );
        std::process::exit(1);
    }

    env::inherit();
    let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
    logs::setup(true, parent_logs != 0, pretty, color);

    let target = &args[0];
    let deps: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();

    for d in &deps {
        assert!(d != target);
    }

    // Build the known dependencies - this requires grabbing locks
    std::env::set_var("REDO_NO_OOB", "1");
    let mut argv = vec!["redo-ifchange".to_string()];
    argv.extend(deps.iter().map(|d| d.to_string()));

    let c_argv: Vec<std::ffi::CString> = argv
        .iter()
        .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
        .collect();

    let rv = spawn_and_wait(&c_argv);
    if rv != 0 {
        std::process::exit(rv);
    }

    // Now rebuild the target itself without locks
    std::env::set_var("REDO_UNLOCKED", "1");
    let argv2 = vec!["redo-ifchange".to_string(), target.clone()];
    let c_argv2: Vec<std::ffi::CString> = argv2
        .iter()
        .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
        .collect();

    let rv = spawn_and_wait(&c_argv2);
    if rv != 0 {
        std::process::exit(rv);
    }
}
