// deps.rs - Dependency checking
//
// Based on redo/deps.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{cycles, env, logs, state};

#[derive(Debug, Clone)]
pub enum DirtyResult {
    Clean,
    Dirty,
    NeedsBuild(Vec<state::File>),
}

impl DirtyResult {
    pub fn is_clean(&self) -> bool {
        matches!(self, DirtyResult::Clean)
    }

    pub fn is_dirty(&self) -> bool {
        !self.is_clean()
    }
}

pub fn isdirty(
    f: &state::File,
    depth: &str,
    max_changed: i64,
    already_checked: &[i64],
    is_checked_fn: Option<&dyn Fn(&state::File) -> bool>,
    set_checked_fn: Option<&dyn Fn(&mut state::File)>,
    log_override_fn: Option<&dyn Fn(&str)>,
) -> Result<DirtyResult, cycles::CyclicDependencyError> {
    let default_is_checked = |f: &state::File| -> bool { f.is_checked() };
    let default_set_checked = |f: &mut state::File| { f.set_checked_save() };
    let default_log_override = |name: &str| { state::warn_override(name) };

    let is_checked = is_checked_fn.unwrap_or(&default_is_checked);
    let set_checked = set_checked_fn.unwrap_or(&default_set_checked);
    let log_override = log_override_fn.unwrap_or(&default_log_override);

    if already_checked.contains(&f.id) {
        return Err(cycles::CyclicDependencyError);
    }

    let mut ac = already_checked.to_vec();
    ac.push(f.id);

    let debug_level = env::with_env(|v| v.debug);
    if debug_level >= 1 {
        logs::debug(&format!(
            "{}?{} {:?},{:?}",
            depth,
            f.nicename(),
            f.is_generated,
            f.is_override
        ));
    }

    if f.failed_runid.is_some() && f.failed_runid.unwrap_or(0) > 0 {
        logs::debug(&format!("{}-- DIRTY (failed last time)", depth));
        return Ok(DirtyResult::Dirty);
    }
    if f.changed_runid.is_none() {
        logs::debug(&format!("{}-- DIRTY (never built)", depth));
        return Ok(DirtyResult::Dirty);
    }
    let changed = f.changed_runid.unwrap();
    if changed > max_changed {
        logs::debug(&format!(
            "{}-- DIRTY (built {} > {}; {:?})",
            depth,
            changed,
            max_changed,
            env::with_env(|v| v.runid)
        ));
        return Ok(DirtyResult::Dirty);
    }
    if is_checked(f) {
        if debug_level >= 1 {
            logs::debug(&format!("{}-- CLEAN (checked)", depth));
        }
        return Ok(DirtyResult::Clean);
    }
    if f.stamp.is_none() {
        logs::debug(&format!("{}-- DIRTY (no stamp)", depth));
        return Ok(DirtyResult::Dirty);
    }

    let newstamp = f.read_stamp();
    if f.stamp.as_deref() != Some(&newstamp) {
        if newstamp == state::STAMP_MISSING {
            logs::debug(&format!("{}-- DIRTY (missing)", depth));
            if f.stamp.is_some() && f.is_generated {
                logs::debug(&format!(
                    "{}   converted target -> source {:?}",
                    depth, f.id
                ));
                let mut f_mut = f.clone();
                f_mut.is_generated = false;
                f_mut.failed_runid = None;
                f_mut.save();
            }
        } else {
            logs::debug(&format!("{}-- DIRTY (mtime)", depth));
        }
        if f.csum.is_some() {
            return Ok(DirtyResult::NeedsBuild(vec![f.clone()]));
        } else {
            return Ok(DirtyResult::Dirty);
        }
    }

    let mut must_build: Vec<state::File> = Vec::new();
    let base = env::with_env(|v| v.base.clone());

    for (mode, f2) in f.deps() {
        let mut dirty = DirtyResult::Clean;

        if mode == "c" {
            let dep_path = format!("{}/{}", base, f2.name);
            if std::path::Path::new(&dep_path).exists() {
                logs::debug(&format!("{}-- DIRTY (created)", depth));
                dirty = DirtyResult::Dirty;
            }
        } else if mode == "m" {
            let new_depth = format!("{}  ", depth);
            let mc = std::cmp::max(changed, f.checked_runid.unwrap_or(0));
            let sub = isdirty(
                &f2,
                &new_depth,
                mc,
                &ac,
                Some(is_checked),
                Some(set_checked),
                Some(log_override),
            )?;
            if sub.is_dirty() {
                logs::debug(&format!("{}-- DIRTY (sub)", depth));
                dirty = sub;
            }
        } else {
            panic!("unknown dep mode: {}", mode);
        }

        if f.csum.is_none() {
            match &dirty {
                DirtyResult::Dirty => return Ok(DirtyResult::Dirty),
                DirtyResult::NeedsBuild(files) => {
                    must_build.extend(files.iter().cloned());
                }
                DirtyResult::Clean => {}
            }
        } else {
            match &dirty {
                DirtyResult::Dirty => return Ok(DirtyResult::NeedsBuild(vec![f.clone()])),
                DirtyResult::NeedsBuild(files) => {
                    must_build.extend(files.iter().cloned());
                }
                DirtyResult::Clean => {}
            }
        }
    }

    if !must_build.is_empty() {
        return Ok(DirtyResult::NeedsBuild(must_build));
    }

    logs::debug(&format!("{}-- CLEAN", depth));

    if f.is_override {
        log_override(&f.name);
    }
    let mut f_mut = f.clone();
    set_checked(&mut f_mut);
    Ok(DirtyResult::Clean)
}
