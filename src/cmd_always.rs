// cmd_always.rs - redo-always: mark target as always out of date
//
// Based on redo/cmd_always.py from apenwarr/redo
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
    f.add_dep("m", state::ALWAYS);
    let mut always = state::File::from_name(state::ALWAYS, true);
    always.stamp = Some(state::STAMP_MISSING.to_string());
    always.set_changed();
    always.save();
    state::commit();
}
