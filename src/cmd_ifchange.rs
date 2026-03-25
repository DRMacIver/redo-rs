// cmd_ifchange.rs - redo-ifchange: build targets if they have changed
//
// Based on redo/cmd_ifchange.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{builder, deps, env, jobserver, logs, state};

fn should_build(t: &str) -> (bool, builder::ShouldBuildResult) {
    let f = state::File::from_name(t, true);
    if f.is_failed() {
        std::process::exit(32);
    }
    let runid = env::with_env(|v| v.runid.unwrap_or(0));
    let dirty = deps::isdirty(&f, "", runid, &[], None, None, None);
    match dirty {
        Ok(deps::DirtyResult::Clean) => (f.is_generated, builder::ShouldBuildResult::Clean),
        Ok(deps::DirtyResult::Dirty) => (f.is_generated, builder::ShouldBuildResult::Dirty),
        Ok(deps::DirtyResult::NeedsBuild(files)) => {
            if files.len() == 1 && files[0].id == f.id {
                (f.is_generated, builder::ShouldBuildResult::Dirty)
            } else {
                (
                    f.is_generated,
                    builder::ShouldBuildResult::NeedsBuild(files),
                )
            }
        }
        Err(_) => (f.is_generated, builder::ShouldBuildResult::Dirty),
    }
}

pub fn main() {
    let rv;
    let targets: Vec<String> = std::env::args().skip(1).collect();
    state::init(&targets);

    let mut effective_targets = targets.clone();
    if env::is_toplevel() && effective_targets.is_empty() {
        effective_targets = vec!["all".to_string()];
    }

    if env::is_toplevel() && env::with_env(|v| v.log) != 0 {
        builder::close_stdin();
        builder::start_stdin_log_reader(true, true, 1, 1, false, false);
    } else {
        let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
        logs::setup(true, parent_logs != 0, pretty, color);
    }

    let (target_env, unlocked) = env::with_env(|v| (v.target.clone(), v.unlocked));
    let f = if !target_env.is_empty() && !unlocked {
        let (startdir, pwd) = env::with_env(|v| (v.startdir.clone(), v.pwd.clone()));
        let me = format!("{}/{}/{}", startdir, pwd, target_env);
        logs::debug2(&format!("TARGET: {:?} {:?} {:?}", startdir, pwd, target_env));
        Some(state::File::from_name(&me, true))
    } else {
        logs::debug2("redo-ifchange: not adding depends.");
        None
    };

    jobserver::setup(0);

    if let Some(ref f) = f {
        for t in &effective_targets {
            f.add_dep("m", t);
        }
        f.save();
        state::commit();
    }

    rv = builder::run(&effective_targets, &should_build);

    state::rollback();
    jobserver::force_return_tokens();
    state::commit();

    if env::is_toplevel() {
        builder::await_log_reader();
    }
    std::process::exit(rv);
}
