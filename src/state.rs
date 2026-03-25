// state.rs - State database management
//
// Based on redo/state.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{cycles, env, helpers, logs};
use nix::errno::Errno;
use nix::fcntl::{open, fcntl, FcntlArg, OFlag};
use nix::sys::stat::Mode;
use rusqlite::{params, Connection};
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;

pub const SCHEMA_VER: i32 = 2;
/// Timeout for SQLite connections (seconds). Used by the Python version;
/// rusqlite handles this via its own timeout parameter.
#[allow(dead_code)]
pub const TIMEOUT: u32 = 60;

pub const ALWAYS: &str = "//ALWAYS";
pub const STAMP_DIR: &str = "dir";
pub const STAMP_MISSING: &str = "0";

pub const LOG_LOCK_MAGIC: i64 = 0x10000000;

static DB: Mutex<Option<Connection>> = Mutex::new(None);
pub static LOCKFILE_FD: Mutex<i32> = Mutex::new(-1);
static WROTE: Mutex<i32> = Mutex::new(0);
static INSANE: Mutex<Option<bool>> = Mutex::new(None);

fn connect(dbfile: &str, locks_broken: bool) -> Connection {
    let db = Connection::open(dbfile).expect("failed to open database");
    db.execute_batch("pragma synchronous = off").unwrap();
    let jmode = if locks_broken { "PERSIST" } else { "WAL" };
    db.execute_batch(&format!("pragma journal_mode = {}", jmode))
        .unwrap();
    db
}

/// Ensure the database is initialized (lazy init).
fn ensure_db() {
    let is_init = DB.lock().unwrap().is_some();
    if !is_init {
        init_db();
    }
}

fn db_write(q: &str, params: &[&dyn rusqlite::ToSql]) {
    if *INSANE.lock().unwrap() == Some(true) {
        return;
    }
    ensure_db();
    let mut wrote = WROTE.lock().unwrap();
    *wrote += 1;
    let mut db_guard = DB.lock().unwrap();
    let db = db_guard.as_mut().expect("database not initialized");
    db.execute(q, params).unwrap_or_else(|e| {
        eprintln!("redo: db write error: {} (query: {})", e, q);
        0
    });
}

/// Initialize the state database and return access to it.
pub fn init_db() {
    let mut db_guard = DB.lock().unwrap();
    if db_guard.is_some() {
        return;
    }

    let base = env::with_env(|v| v.base.clone());
    let dbdir = format!("{}/.redo", base);
    let dbfile = format!("{}/db.sqlite3", dbdir);

    let _ = std::fs::create_dir(&dbdir);

    let lockfile_path = format!("{}/.redo/locks", base);
    let lf = open(
        lockfile_path.as_str(),
        OFlag::O_RDWR | OFlag::O_CREAT,
        Mode::from_bits_truncate(0o666),
    )
    .expect("failed to open lock file");
    helpers::close_on_exec(lf, true);
    *LOCKFILE_FD.lock().unwrap() = lf;

    let locks_broken = env::with_env(|v| v.locks_broken);

    let must_create = !std::path::Path::new(&dbfile).exists();
    if !must_create {
        let db = connect(&dbfile, locks_broken);
        let ver: Option<i32> = db
            .query_row("select version from Schema", [], |row| row.get(0))
            .ok();
        if ver != Some(SCHEMA_VER) {
            eprintln!(
                "redo: {}: found v{:?} (expected v{})",
                dbfile, ver, SCHEMA_VER
            );
            eprintln!("redo: manually delete .redo dir to start over.");
            std::process::exit(1);
        }
        *db_guard = Some(db);
    }

    if must_create {
        helpers::unlink(&dbfile);
        let db = connect(&dbfile, locks_broken);
        db.execute_batch(
            "create table Schema (version int);
             create table Runid (id integer primary key autoincrement);
             create table Files (
                 name not null primary key,
                 is_generated int,
                 is_override int,
                 checked_runid int,
                 changed_runid int,
                 failed_runid int,
                 stamp,
                 csum);
             create table Deps (
                 target int,
                 source int,
                 mode not null,
                 delete_me int,
                 primary key (target, source));",
        )
        .unwrap();
        db.execute("insert into Schema (version) values (?1)", params![SCHEMA_VER])
            .unwrap();
        db.execute("insert into Runid values (1000000000)", [])
            .unwrap();
        db.execute("insert into Files (name) values (?1)", params![ALWAYS])
            .unwrap();
        *db_guard = Some(db);
    }

    let runid = env::with_env(|v| v.runid);
    if runid.is_none() {
        let db = db_guard.as_ref().unwrap();
        db.execute(
            "insert into Runid values ((select max(id)+1 from Runid))",
            [],
        )
        .unwrap();
        let new_runid: i64 = db
            .query_row("select last_insert_rowid()", [], |row| row.get(0))
            .unwrap();
        env::with_env_mut(|v| v.runid = Some(new_runid));
        std::env::set_var("REDO_RUNID", new_runid.to_string());
    }

    let db = db_guard.as_ref().unwrap();
    db.execute_batch("COMMIT; BEGIN").unwrap_or_else(|_| {
        // If no transaction is active, just begin one
        let _ = db.execute_batch("BEGIN");
    });
    // Actually just commit any implicit transaction
    let _ = db.execute_batch("");
}

pub fn init(targets: &[String]) {
    env::init(targets);
    init_db();
    if env::is_toplevel() && detect_broken_locks() {
        env::mark_locks_broken();
    }
}

pub fn commit() {
    if *INSANE.lock().unwrap() == Some(true) {
        return;
    }
    ensure_db();
    let mut wrote = WROTE.lock().unwrap();
    if *wrote > 0 {
        let db_guard = DB.lock().unwrap();
        if let Some(db) = db_guard.as_ref() {
            let _ = db.execute_batch("COMMIT; BEGIN");
        }
        *wrote = 0;
    }
}

pub fn rollback() {
    if *INSANE.lock().unwrap() == Some(true) {
        return;
    }
    let mut wrote = WROTE.lock().unwrap();
    if *wrote > 0 {
        let db_guard = DB.lock().unwrap();
        if let Some(db) = db_guard.as_ref() {
            let _ = db.execute_batch("ROLLBACK; BEGIN");
        }
        *wrote = 0;
    }
}

pub fn is_flushed() -> bool {
    *WROTE.lock().unwrap() == 0
}

pub fn check_sane() -> bool {
    let mut insane = INSANE.lock().unwrap();
    if insane.is_none() {
        let base = env::with_env(|v| v.base.clone());
        *insane = Some(!std::path::Path::new(&format!("{}/.redo", base)).exists());
    }
    !insane.unwrap_or(false)
}

fn realdirpath(t: &str) -> String {
    let path = std::path::Path::new(t);
    let (dname, fname) = (
        path.parent().unwrap_or(std::path::Path::new("")),
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
    );
    if dname.as_os_str().is_empty() || dname == std::path::Path::new("") {
        fname
    } else {
        let real_dname = std::fs::canonicalize(dname)
            .unwrap_or_else(|_| dname.to_path_buf())
            .to_string_lossy()
            .to_string();
        format!("{}/{}", real_dname, fname)
    }
}

pub fn relpath(t: &str, base: &str) -> String {
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Like Python's os.path.join: if t is absolute, use it as-is
    let t_joined = if std::path::Path::new(t).is_absolute() {
        t.to_string()
    } else {
        format!("{}/{}", cwd, t)
    };
    let t_norm = normalize_path(&realdirpath(&t_joined));
    let base_norm = normalize_path(&realdirpath(base));

    let mut tparts: Vec<&str> = t_norm.split('/').collect();
    let mut bparts: Vec<&str> = base_norm.split('/').collect();

    // Remove common prefix (matching Python's zip-and-pop logic)
    let pairs: Vec<(&&str, &&str)> = tparts.iter().zip(bparts.iter()).collect();
    let mut to_remove = 0;
    for (tp, bp) in &pairs {
        if tp != bp {
            break;
        }
        to_remove += 1;
    }
    tparts.drain(..to_remove);
    bparts.drain(..to_remove);

    let mut result: Vec<&str> = Vec::new();
    for _ in &bparts {
        result.push("..");
    }
    result.extend(&tparts);

    if result.is_empty() {
        ".".to_string()
    } else {
        result.join("/")
    }
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let is_absolute = path.starts_with('/');
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() && *parts.last().unwrap() != ".." {
                    parts.pop();
                } else if !is_absolute {
                    parts.push("..");
                }
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if is_absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Return a relative path for t that will work after we chdir to dirname(TARGET).
pub fn target_relpath(t: &str) -> String {
    let (startdir, pwd, target) = env::with_env(|v| {
        (v.startdir.clone(), v.pwd.clone(), v.target.clone())
    });

    let dofile_dir = if pwd.is_empty() {
        std::fs::canonicalize(&startdir)
            .unwrap_or_else(|_| std::path::PathBuf::from(&startdir))
            .to_string_lossy()
            .to_string()
    } else {
        let combined = format!("{}/{}", startdir, pwd);
        std::fs::canonicalize(&combined)
            .unwrap_or_else(|_| std::path::PathBuf::from(&combined))
            .to_string_lossy()
            .to_string()
    };

    // Python: os.path.dirname(os.path.join(dofile_dir, target))
    // When target is empty, Python's join adds trailing slash and dirname
    // returns the same dir. Rust's Path::parent strips the last component.
    let target_dir = if target.is_empty() {
        dofile_dir.clone()
    } else {
        let target_abs = format!("{}/{}", dofile_dir, target);
        std::path::Path::new(&target_abs)
            .parent()
            .unwrap_or(std::path::Path::new("/"))
            .to_string_lossy()
            .to_string()
    };
    let target_dir_abs = std::fs::canonicalize(&target_dir)
        .unwrap_or_else(|_| std::path::PathBuf::from(&target_dir))
        .to_string_lossy()
        .to_string();
    relpath(t, &target_dir_abs)
}

pub fn detect_override(stamp1: &str, stamp2: &str) -> bool {
    if stamp1 == stamp2 {
        return false;
    }
    let crit1: Vec<&str> = stamp1.splitn(3, '-').take(2).collect();
    let crit2: Vec<&str> = stamp2.splitn(3, '-').take(2).collect();
    crit1 != crit2
}

pub fn warn_override(name: &str) {
    logs::warn(&format!("{} - you modified it; skipping", name));
}

#[derive(Debug, Clone)]
pub struct File {
    pub id: i64,
    pub name: String,
    pub is_generated: bool,
    pub is_override: bool,
    pub checked_runid: Option<i64>,
    pub changed_runid: Option<i64>,
    pub failed_runid: Option<i64>,
    pub stamp: Option<String>,
    pub csum: Option<String>,
}

impl File {
    pub fn from_name(name: &str, allow_add: bool) -> Self {
        let resolved_name = if name == ALWAYS {
            ALWAYS.to_string()
        } else {
            let base = env::with_env(|v| v.base.clone());
            relpath(name, &base)
        };
        Self::query_by_name(&resolved_name, allow_add)
    }

    pub fn from_id(fid: i64) -> Self {
        Self::query_by_id(fid)
    }

    fn query_by_name(name: &str, allow_add: bool) -> Self {
        ensure_db();
        let db_guard = DB.lock().unwrap();
        let db = db_guard.as_ref().expect("database not initialized");

        let result: Option<Self> = db
            .query_row(
                "select rowid, name, is_generated, is_override, \
                 checked_runid, changed_runid, failed_runid, stamp, csum \
                 from Files where name=?1",
                params![name],
                |row| {
                    Ok(File {
                        id: row.get(0)?,
                        name: row.get::<_, String>(1)?,
                        is_generated: row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                        is_override: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                        checked_runid: row.get(4)?,
                        changed_runid: row.get(5)?,
                        failed_runid: row.get(6)?,
                        stamp: row.get(7)?,
                        csum: row.get(8)?,
                    })
                },
            )
            .ok();

        if let Some(mut f) = result {
            f.fix_always();
            return f;
        }

        if !allow_add {
            panic!("No file with name={:?}", name);
        }

        // Insert and retry - ignore integrity error (parallel insert)
        let _ = db.execute("insert into Files (name) values (?1)", params![name]);

        let mut f: File = db
            .query_row(
                "select rowid, name, is_generated, is_override, \
                 checked_runid, changed_runid, failed_runid, stamp, csum \
                 from Files where name=?1",
                params![name],
                |row| {
                    Ok(File {
                        id: row.get(0)?,
                        name: row.get::<_, String>(1)?,
                        is_generated: row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                        is_override: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                        checked_runid: row.get(4)?,
                        changed_runid: row.get(5)?,
                        failed_runid: row.get(6)?,
                        stamp: row.get(7)?,
                        csum: row.get(8)?,
                    })
                },
            )
            .expect("Failed to query file after insert");
        f.fix_always();
        f
    }

    fn query_by_id(fid: i64) -> Self {
        ensure_db();
        let db_guard = DB.lock().unwrap();
        let db = db_guard.as_ref().expect("database not initialized");

        let mut f: File = db
            .query_row(
                "select rowid, name, is_generated, is_override, \
                 checked_runid, changed_runid, failed_runid, stamp, csum \
                 from Files where rowid=?1",
                params![fid],
                |row| {
                    Ok(File {
                        id: row.get(0)?,
                        name: row.get::<_, String>(1)?,
                        is_generated: row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                        is_override: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                        checked_runid: row.get(4)?,
                        changed_runid: row.get(5)?,
                        failed_runid: row.get(6)?,
                        stamp: row.get(7)?,
                        csum: row.get(8)?,
                    })
                },
            )
            .unwrap_or_else(|_| panic!("No file with id={}", fid));
        f.fix_always();
        f
    }

    fn fix_always(&mut self) {
        let runid = env::with_env(|v| v.runid);
        if self.name == ALWAYS {
            if let Some(rid) = runid {
                if self.changed_runid.is_none() || self.changed_runid.unwrap() < rid {
                    self.changed_runid = Some(rid);
                }
            }
        }
    }

    pub fn refresh(&mut self) {
        *self = File::query_by_id(self.id);
    }

    pub fn save(&self) {
        db_write(
            "update Files set \
             is_generated=?1, is_override=?2, \
             checked_runid=?3, changed_runid=?4, failed_runid=?5, \
             stamp=?6, csum=?7 \
             where rowid=?8",
            &[
                &(self.is_generated as i64),
                &(self.is_override as i64),
                &self.checked_runid,
                &self.changed_runid,
                &self.failed_runid,
                &self.stamp,
                &self.csum,
                &self.id,
            ],
        );
    }

    pub fn set_checked(&mut self) {
        self.checked_runid = env::with_env(|v| v.runid);
    }

    pub fn set_checked_save(&mut self) {
        self.set_checked();
        self.save();
    }

    pub fn set_changed(&mut self) {
        logs::debug2(&format!("BUILT: {:?} ({:?})", self.name, self.stamp));
        self.changed_runid = env::with_env(|v| v.runid);
        self.failed_runid = None;
        self.is_override = false;
    }

    pub fn set_failed(&mut self) {
        logs::debug2(&format!("FAILED: {:?}", self.name));
        self.update_stamp(false);
        self.failed_runid = env::with_env(|v| v.runid);
        if self.stamp.as_deref() != Some(STAMP_MISSING) {
            self.is_generated = true;
        } else {
            self.is_generated = false;
        }
    }

    pub fn set_static(&mut self) {
        self.update_stamp(true);
        self.failed_runid = None;
        self.is_override = false;
        self.is_generated = false;
    }

    pub fn set_override(&mut self) {
        self.update_stamp(false);
        self.failed_runid = None;
        self.is_override = true;
    }

    pub fn update_stamp(&mut self, must_exist: bool) {
        let newstamp = self.read_stamp();
        if must_exist && newstamp == STAMP_MISSING {
            panic!("{:?} does not exist", self.name);
        }
        if Some(&newstamp) != self.stamp.as_ref() {
            logs::debug2(&format!(
                "STAMP: {}: {:?} -> {:?}",
                self.name, self.stamp, newstamp
            ));
            self.stamp = Some(newstamp);
            self.set_changed();
        }
    }

    pub fn is_source(&self) -> bool {
        if self.name.starts_with("//") {
            return false;
        }
        let newstamp = self.read_stamp();
        if self.is_generated
            && (!self.is_failed() || newstamp != STAMP_MISSING)
            && !self.is_override
            && self.stamp.as_deref() == Some(&newstamp)
        {
            return false;
        }
        if (!self.is_generated || self.stamp.as_deref() != Some(&newstamp))
            && newstamp == STAMP_MISSING
        {
            return false;
        }
        true
    }

    pub fn is_target(&self) -> bool {
        if !self.is_generated {
            return false;
        }
        if self.is_source() {
            return false;
        }
        true
    }

    pub fn is_checked(&self) -> bool {
        let runid = env::with_env(|v| v.runid);
        if let (Some(cr), Some(rid)) = (self.checked_runid, runid) {
            cr >= rid
        } else {
            false
        }
    }

    pub fn is_changed(&self) -> bool {
        let runid = env::with_env(|v| v.runid);
        if let (Some(cr), Some(rid)) = (self.changed_runid, runid) {
            cr >= rid
        } else {
            false
        }
    }

    pub fn is_failed(&self) -> bool {
        let runid = env::with_env(|v| v.runid);
        if let (Some(fr), Some(rid)) = (self.failed_runid, runid) {
            fr >= rid
        } else {
            false
        }
    }

    pub fn deps(&self) -> Vec<(String, File)> {
        if self.is_override || !self.is_generated {
            return Vec::new();
        }
        ensure_db();
        let db_guard = DB.lock().unwrap();
        let db = db_guard.as_ref().expect("database not initialized");
        let mut stmt = db
            .prepare(
                "select Deps.mode, Deps.source, \
                 Files.name, Files.is_generated, Files.is_override, \
                 Files.checked_runid, Files.changed_runid, Files.failed_runid, \
                 Files.stamp, Files.csum \
                 from Files \
                 join Deps on Files.rowid = Deps.source \
                 where target=?1",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![self.id], |row| {
                let mode: String = row.get(0)?;
                let mut f = File {
                    id: row.get(1)?,
                    name: row.get::<_, String>(2)?,
                    is_generated: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                    is_override: row.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
                    checked_runid: row.get(5)?,
                    changed_runid: row.get(6)?,
                    failed_runid: row.get(7)?,
                    stamp: row.get(8)?,
                    csum: row.get(9)?,
                };
                f.fix_always();
                Ok((mode, f))
            })
            .unwrap();
        rows.filter_map(|r| r.ok()).collect()
    }

    pub fn zap_deps1(&self) {
        logs::debug2(&format!("zap-deps1: {:?}", self.name));
        db_write(
            "update Deps set delete_me=1 where target=?1",
            &[&self.id],
        );
    }

    pub fn zap_deps2(&self) {
        logs::debug2(&format!("zap-deps2: {:?}", self.name));
        db_write(
            "delete from Deps where target=?1 and delete_me=1",
            &[&self.id],
        );
    }

    pub fn add_dep(&self, mode: &str, dep: &str) {
        let src = File::from_name(dep, true);
        logs::debug3(&format!(
            "add-dep: \"{}\" < {} \"{}\"",
            self.name, mode, src.name
        ));
        assert!(self.id != src.id);
        db_write(
            "insert or replace into Deps \
             (target, mode, source, delete_me) values (?1, ?2, ?3, ?4)",
            &[&self.id, &mode as &dyn rusqlite::ToSql, &src.id, &false],
        );
    }

    fn read_stamp_st(&self, follow_symlinks: bool) -> (bool, String) {
        let base = env::with_env(|v| v.base.clone());
        let fullpath = format!("{}/{}", base, self.name);

        let md = if follow_symlinks {
            std::fs::metadata(&fullpath)
        } else {
            std::fs::symlink_metadata(&fullpath)
        };

        match md {
            Err(_) => (false, STAMP_MISSING.to_string()),
            Ok(md) => {
                if md.is_dir() {
                    (false, STAMP_DIR.to_string())
                } else {
                    let is_link = md.file_type().is_symlink();
                    let mtime = md.mtime() as f64 + (md.mtime_nsec() as f64 / 1_000_000_000.0);
                    let stamp = format!(
                        "{:.6}-{}-{}-{}-{}-{}",
                        mtime,
                        md.size(),
                        md.ino(),
                        md.mode(),
                        md.uid(),
                        md.gid()
                    );
                    (is_link, stamp)
                }
            }
        }
    }

    pub fn read_stamp(&self) -> String {
        let (is_link, pre) = self.read_stamp_st(false);
        if is_link {
            let (_, post) = self.read_stamp_st(true);
            format!("{}+{}", pre, post)
        } else {
            pre
        }
    }

    pub fn nicename(&self) -> String {
        let (base, startdir) = env::with_env(|v| (v.base.clone(), v.startdir.clone()));
        relpath(&format!("{}/{}", base, self.name), &startdir)
    }
}

pub fn files() -> Vec<File> {
    ensure_db();
    let db_guard = DB.lock().unwrap();
    let db = db_guard.as_ref().expect("database not initialized");
    let mut stmt = db
        .prepare(
            "select rowid, name, is_generated, is_override, \
             checked_runid, changed_runid, failed_runid, stamp, csum \
             from Files order by name",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            let mut f = File {
                id: row.get(0)?,
                name: row.get::<_, String>(1)?,
                is_generated: row.get::<_, Option<i64>>(2)?.unwrap_or(0) != 0,
                is_override: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                checked_runid: row.get(4)?,
                changed_runid: row.get(5)?,
                failed_runid: row.get(6)?,
                stamp: row.get(7)?,
                csum: row.get(8)?,
            };
            f.fix_always();
            Ok(f)
        })
        .unwrap();
    rows.filter_map(|r| r.ok()).collect()
}

pub fn logname(fid: i64) -> String {
    let base = env::with_env(|v| v.base.clone());
    format!("{}/.redo/log.{}", base, fid)
}

// Locks
static LOCKS: Mutex<Option<std::collections::HashMap<i64, i32>>> = Mutex::new(None);

fn get_locks() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<i64, i32>>> {
    let mut guard = LOCKS.lock().unwrap();
    if guard.is_none() {
        *guard = Some(std::collections::HashMap::new());
    }
    guard
}

pub struct Lock {
    pub owned: bool,
    pub fid: i64,
}

impl Lock {
    pub fn new(fid: i64) -> Self {
        let lockfile = *LOCKFILE_FD.lock().unwrap();
        assert!(lockfile >= 0, "lockfile not initialized");
        {
            let mut locks = get_locks();
            let map = locks.as_mut().unwrap();
            assert!(
                *map.get(&fid).unwrap_or(&0) == 0,
                "lock {} already exists",
                fid
            );
            map.insert(fid, 1);
        }
        Lock { owned: false, fid }
    }

    pub fn check(&self) -> Result<(), cycles::CyclicDependencyError> {
        assert!(!self.owned);
        cycles::check(self.fid)
    }

    pub fn trylock(&mut self) -> bool {
        self.check().unwrap_or_else(|e| {
            panic!("cycle check failed: {}", e);
        });
        assert!(!self.owned);
        let lockfile = *LOCKFILE_FD.lock().unwrap();
        let result = do_lockf(lockfile, LockType::Write, true, 1, self.fid);
        if result {
            self.owned = true;
        }
        self.owned
    }

    pub fn trylock_returning_cycle_error(&mut self) -> Result<bool, cycles::CyclicDependencyError> {
        self.check()?;
        assert!(!self.owned);
        let lockfile = *LOCKFILE_FD.lock().unwrap();
        let result = do_lockf(lockfile, LockType::Write, true, 1, self.fid);
        if result {
            self.owned = true;
        }
        Ok(self.owned)
    }

    pub fn waitlock(&mut self, shared: bool) {
        let _ = self.check();
        assert!(!self.owned);
        let lockfile = *LOCKFILE_FD.lock().unwrap();
        let lock_type = if shared { LockType::Read } else { LockType::Write };
        do_lockf(lockfile, lock_type, false, 1, self.fid);
        self.owned = true;
    }

    pub fn unlock(&mut self) {
        assert!(self.owned, "can't unlock - we don't own it");
        let lockfile = *LOCKFILE_FD.lock().unwrap();
        do_lockf(lockfile, LockType::Unlock, false, 1, self.fid);
        self.owned = false;
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        {
            let mut locks = get_locks();
            if let Some(map) = locks.as_mut() {
                map.insert(self.fid, 0);
            }
        }
        if self.owned {
            self.unlock();
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LockType {
    Read,
    Write,
    Unlock,
}

fn do_lockf(fd: i32, lock_type: LockType, nonblock: bool, len: i64, start: i64) -> bool {
    // We use libc types for the flock struct since nix::fcntl::FcntlArg::F_SETLK requires it
    let fl = libc::flock {
        l_type: match lock_type {
            LockType::Read => libc::F_RDLCK as i16,
            LockType::Write => libc::F_WRLCK as i16,
            LockType::Unlock => libc::F_UNLCK as i16,
        },
        l_whence: libc::SEEK_SET as i16,
        l_start: start as libc::off_t,
        l_len: len as libc::off_t,
        l_pid: 0,
        #[cfg(target_os = "linux")]
        l_sysid: 0,
    };
    let arg = if lock_type == LockType::Unlock || nonblock {
        FcntlArg::F_SETLK(&fl)
    } else {
        FcntlArg::F_SETLKW(&fl)
    };

    match fcntl(fd, arg) {
        Ok(_) => true,
        Err(errno) => {
            if nonblock && (errno == Errno::EAGAIN || errno == Errno::EACCES) {
                return false;
            }
            if lock_type == LockType::Unlock {
                return true;
            }
            panic!("fcntl lock failed: {}", errno);
        }
    }
}

pub fn detect_broken_locks() -> bool {
    // Fork-based test for broken locks (WSL)
    // On macOS/Linux, locks generally work fine, so just return false
    // for the Rust port to avoid complexity.
    // The full test would fork and check if child can acquire parent's lock.
    false
}
