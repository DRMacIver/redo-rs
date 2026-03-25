// cmd_ifcreate.rs - redo-ifcreate: rebuild if these targets are created
//
// Based on redo/cmd_ifcreate.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, logs, state};

pub fn main() {
    env::inherit();
    let (parent_logs, pretty, color) = env::with_env(|v| (v.log, v.pretty, v.color));
    logs::setup(true, parent_logs != 0, pretty, color);

    let (startdir, pwd, target) = env::with_env(|v| {
        (v.startdir.clone(), v.pwd.clone(), v.target.clone())
    });
    let me = format!("{}/{}/{}", startdir, pwd, target);
    let f = state::File::from_name(&me, true);

    for t in std::env::args().skip(1) {
        if t.is_empty() {
            logs::err("cannot build the empty target (\"\").");
            std::process::exit(204);
        }
        if std::path::Path::new(&t).exists() {
            logs::err(&format!("redo-ifcreate: error: {:?} already exists", t));
            std::process::exit(1);
        } else {
            f.add_dep("c", &t);
        }
    }
    state::commit();
}
