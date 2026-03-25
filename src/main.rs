// main.rs - Multi-command dispatcher for redo
//
// This is a Rust port of apenwarr/redo (https://github.com/apenwarr/redo)
// Original implementation: Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

mod builder;
mod cmd_always;
mod cmd_ifchange;
mod cmd_ifcreate;
mod cmd_log;
mod cmd_ood;
mod cmd_redo;
mod cmd_sources;
mod cmd_stamp;
mod cmd_targets;
mod cmd_unlocked;
mod cmd_whichdo;
mod cycles;
mod deps;
mod env;
mod helpers;
mod jobserver;
mod logs;
mod options;
mod paths;
mod state;

fn main() {
    let argv0 = std::env::args().next().unwrap_or_default();
    let cmd = std::path::Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("redo");

    match cmd {
        "redo" => cmd_redo::main(),
        "redo-ifchange" => cmd_ifchange::main(),
        "redo-ifcreate" => cmd_ifcreate::main(),
        "redo-always" => cmd_always::main(),
        "redo-stamp" => cmd_stamp::main(),
        "redo-log" => cmd_log::main(),
        "redo-whichdo" => cmd_whichdo::main(),
        "redo-targets" => cmd_targets::main(),
        "redo-sources" => cmd_sources::main(),
        "redo-ood" => cmd_ood::main(),
        "redo-unlocked" => cmd_unlocked::main(),
        _ => {
            eprintln!("redo: unknown command: {}", cmd);
            std::process::exit(1);
        }
    }
}
