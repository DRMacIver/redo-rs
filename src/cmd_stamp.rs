// cmd_stamp.rs - redo-stamp: use checksum for change detection
//
// Based on redo/cmd_stamp.py from apenwarr/redo
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
        eprintln!("{}: no arguments expected.", std::env::args().next().unwrap_or_default());
        std::process::exit(1);
    }

    if nix::unistd::isatty(0).unwrap_or(false) {
        eprintln!(
            "{}: you must provide the data to stamp on stdin",
            std::env::args().next().unwrap_or_default()
        );
        std::process::exit(1);
    }

    env::inherit();
    let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
    logs::setup(true, parent_logs != 0, pretty, color);

    let mut hasher = sha1_smol::Sha1::new();
    let mut buf = [0u8; 4096];
    loop {
        match nix::unistd::read(0, &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => hasher.update(&buf[..n]),
        }
    }
    let csum = hasher.digest().to_string();

    let target = env::with_env(|v| v.target.clone());
    if target.is_empty() {
        std::process::exit(0);
    }

    let (startdir, pwd) = env::with_env(|v| (v.startdir.clone(), v.pwd.clone()));
    let me = format!("{}/{}/{}", startdir, pwd, target);
    let mut f = state::File::from_name(&me, true);

    let changed = f.csum.as_deref() != Some(&csum);
    logs::debug2(&format!("{}: old = {:?}", f.name, f.csum));
    logs::debug2(&format!(
        "{}: sum = {} ({})",
        f.name,
        csum,
        if changed { "changed" } else { "unchanged" }
    ));

    f.is_generated = true;
    f.is_override = false;
    f.failed_runid = None;
    if changed {
        f.set_changed();
        f.csum = Some(csum);
    } else {
        f.set_checked();
    }
    f.save();
    state::commit();
}
