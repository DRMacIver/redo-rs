// cmd_redo.rs - redo command: build targets whether they need it or not
//
// Based on redo/cmd_redo.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{builder, env, jobserver, logs, options, state};

const OPTSPEC: &str = "\
redo [targets...]
--
j,jobs=    maximum number of jobs to build at once
d,debug    print dependency checks as they happen
v,verbose  print commands as they are read from .do files (variables intact)
x,xtrace   print commands as they are executed (variables expanded)
k,keep-going  keep going as long as possible even if some targets fail
shuffle    randomize the build order to find dependency bugs
version    print the current version and exit

 redo-log options:
no-log     don't capture error output, just let it flow straight to stderr
no-details only show 'redo' recursion trace (to see more later, use redo-log)
no-status  don't display build summary line at the bottom of the screen
no-pretty  don't pretty-print logs, show raw @@REDO output instead
no-color   disable ANSI color; --color to force enable (default: auto)
debug-locks  print messages about file locking (useful for debugging)
debug-pids   print process ids as part of log messages (useful for debugging)";

pub fn main() {
    let o = options::Options::new(OPTSPEC);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (opt, mut targets) = o.parse(&args);

    if opt.bool_val("version") {
        println!("redo-rs 0.1.0 (port of apenwarr/redo)");
        std::process::exit(0);
    }
    if opt.bool_val("debug") {
        std::env::set_var("REDO_DEBUG", opt.int_val("debug").to_string());
    }
    if opt.bool_val("verbose") {
        std::env::set_var("REDO_VERBOSE", opt.int_val("verbose").to_string());
    }
    if opt.bool_val("xtrace") {
        std::env::set_var("REDO_XTRACE", opt.int_val("xtrace").to_string());
    }
    if opt.bool_val("keep_going") {
        std::env::set_var("REDO_KEEP_GOING", "1");
    }
    if opt.bool_val("shuffle") {
        std::env::set_var("REDO_SHUFFLE", "1");
    }
    if opt.bool_val("debug_locks") {
        std::env::set_var("REDO_DEBUG_LOCKS", "1");
    }
    if opt.bool_val("debug_pids") {
        std::env::set_var("REDO_DEBUG_PIDS", "1");
    }

    // Set defaults for log options
    fn set_defint(name: &str, val: i64) {
        if std::env::var(name).is_err() {
            std::env::set_var(name, val.to_string());
        }
    }
    set_defint("REDO_LOG", opt.int_val("log"));
    set_defint("REDO_PRETTY", opt.int_val("pretty"));
    set_defint("REDO_COLOR", opt.int_val("color"));

    state::init(&targets);
    if env::is_toplevel() && targets.is_empty() {
        targets = vec!["all".to_string()];
    }

    let j = opt.int_val("jobs") as i32;
    let log_val = env::with_env(|v| v.log);

    if env::is_toplevel() && (log_val != 0 || j > 1) {
        builder::close_stdin();
    }
    if env::is_toplevel() && log_val != 0 {
        let pretty = env::with_env(|v| v.pretty);
        let color = env::with_env(|v| v.color);
        builder::start_stdin_log_reader(
            opt.bool_val("status"),
            opt.bool_val("details"),
            pretty,
            color,
            opt.bool_val("debug_locks"),
            opt.bool_val("debug_pids"),
        );
    } else {
        let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
        logs::setup(true, parent_logs != 0, pretty, color);
    }

    let locks_broken = env::with_env(|v| v.locks_broken);
    if (env::is_toplevel() || j > 1) && locks_broken {
        logs::warn("detected broken fcntl locks; parallelism disabled.");
    }

    // Warn about existing non-generated files
    for t in &targets {
        if std::path::Path::new(t).exists() {
            let f = state::File::from_name(t, true);
            if !f.is_generated {
                logs::warn(&format!(
                    "{}: exists and not marked as generated; not redoing.",
                    f.nicename()
                ));
            }
        }
    }
    state::rollback();

    let j = if j < 0 || j > 1000 {
        logs::err(&format!("invalid --jobs value: {:?}", opt.str_val("jobs")));
        0
    } else {
        j
    };
    jobserver::setup(j);

    let retcode;
    {
        assert!(state::is_flushed());
        retcode = builder::run(&targets, &|_t: &str| {
            (true, builder::ShouldBuildResult::Dirty)
        });
        assert!(state::is_flushed());
    }

    state::rollback();
    jobserver::force_return_tokens();

    if env::is_toplevel() {
        builder::await_log_reader();
    }
    std::process::exit(retcode);
}
