// builder.rs - Build orchestration
//
// Based on redo/builder.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{cycles, env, helpers, jobserver, logs, paths, state};
use std::io::Write;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};

use nix::fcntl::{fcntl, FcntlArg, FdFlag};
use nix::sys::signal::{signal, SigHandler, Signal};
use nix::sys::stat::fstat;
use nix::unistd::{dup, dup2, fork, isatty, lseek, pipe, ForkResult, Whence};

/// Wrapper to make a raw pointer Send. This is safe because the pointer
/// is only used after fork() in a single-threaded context (the child
/// never actually uses it; it's used by the parent in donefunc callbacks
/// which run sequentially in the parent process).
struct SendPtr(*mut i32);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

fn nice(t: &str) -> String {
    let startdir = env::with_env(|v| v.startdir.clone());
    state::relpath(t, &startdir)
}

fn try_stat(filename: &str) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(filename).ok()
}

static LOG_READER_PID: std::sync::Mutex<Option<i32>> = std::sync::Mutex::new(None);
static STDERR_FD: std::sync::Mutex<Option<i32>> = std::sync::Mutex::new(None);

pub fn close_stdin() {
    let f = std::fs::File::open("/dev/null").unwrap();
    let fd = f.as_raw_fd();
    dup2(fd, 0).expect("dup2 failed");
}

pub fn start_stdin_log_reader(
    status: bool,
    details: bool,
    pretty: i64,
    color: i64,
    debug_locks: bool,
    debug_pids: bool,
) {
    let (pipe_r, pipe_w) = pipe().expect("pipe() failed");
    let r = pipe_r.into_raw_fd();
    let w = pipe_w.into_raw_fd();

    let (apipe_r, apipe_w) = pipe().expect("pipe() failed");
    let ar = apipe_r.into_raw_fd();
    let aw = apipe_w.into_raw_fd();

    // make aw inheritable
    {
        let flags = fcntl(aw, FcntlArg::F_GETFD).expect("fcntl F_GETFD failed");
        let new_flags = FdFlag::from_bits_truncate(flags) & !FdFlag::FD_CLOEXEC;
        fcntl(aw, FcntlArg::F_SETFD(new_flags)).expect("fcntl F_SETFD failed");
    }

    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    match unsafe { fork() }.expect("fork failed") {
        ForkResult::Parent { child } => {
            // parent
            *LOG_READER_PID.lock().unwrap() = Some(child.as_raw());
            let saved_stderr = dup(2).expect("dup(2) failed");
            *STDERR_FD.lock().unwrap() = Some(saved_stderr);
            drop(unsafe { OwnedFd::from_raw_fd(r) });
            drop(unsafe { OwnedFd::from_raw_fd(aw) });

            // Wait for ack from child
            let mut buf = [0u8; 8];
            let n = nix::unistd::read(ar, &mut buf).unwrap_or(0);
            if n == 0 {
                logs::err("failed to start redo-log subprocess; cannot continue.");
                std::process::exit(99);
            }
            assert_eq!(&buf[..n], b"REDO-OK\n");

            drop(unsafe { OwnedFd::from_raw_fd(ar) });
            dup2(w, 1).expect("dup2 failed");
            dup2(w, 2).expect("dup2 failed");
            drop(unsafe { OwnedFd::from_raw_fd(w) });
            logs::setup(true, true, 0, 0);
        }
        ForkResult::Child => {
            // child
            drop(unsafe { OwnedFd::from_raw_fd(ar) });
            drop(unsafe { OwnedFd::from_raw_fd(w) });
            dup2(r, 0).expect("dup2 failed");
            drop(unsafe { OwnedFd::from_raw_fd(r) });
            dup2(2, 1).expect("dup2 failed");

            let is_tty = isatty(2).unwrap_or(false);
            let mut argv: Vec<String> = vec![
                "redo-log".to_string(),
                "--recursive".to_string(),
                "--follow".to_string(),
                "--ack-fd".to_string(),
                aw.to_string(),
                if status && is_tty {
                    "--status".to_string()
                } else {
                    "--no-status".to_string()
                },
                if details {
                    "--details".to_string()
                } else {
                    "--no-details".to_string()
                },
                if pretty != 0 {
                    "--pretty".to_string()
                } else {
                    "--no-pretty".to_string()
                },
                if debug_locks {
                    "--debug-locks".to_string()
                } else {
                    "--no-debug-locks".to_string()
                },
                if debug_pids {
                    "--debug-pids".to_string()
                } else {
                    "--no-debug-pids".to_string()
                },
            ];
            if color != 1 {
                argv.push(if color >= 2 {
                    "--color".to_string()
                } else {
                    "--no-color".to_string()
                });
            }
            argv.push("-".to_string());

            let c_argv: Vec<std::ffi::CString> = argv
                .iter()
                .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
                .collect();

            let _ = nix::unistd::execvp(&c_argv[0], &c_argv);
            eprintln!("redo-log: exec failed");
            std::process::exit(99);
        }
    }
}

pub fn await_log_reader() {
    let log = env::with_env(|v| v.log);
    if log == 0 {
        return;
    }
    let pid = *LOG_READER_PID.lock().unwrap();
    if let Some(pid) = pid {
        if pid > 0 {
            let stderr_fd = STDERR_FD.lock().unwrap().unwrap_or(2);
            dup2(stderr_fd, 1).expect("dup2 failed");
            dup2(stderr_fd, 2).expect("dup2 failed");
            let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(pid), None);
        }
    }
}

/// The main build function. Build the given targets, if necessary.
pub fn run(
    targets: &[String],
    shouldbuildfunc: &dyn Fn(&str) -> (bool, ShouldBuildResult),
) -> i32 {
    let mut retcode = 0;
    let shuffle = env::with_env(|v| v.shuffle);

    let mut targets = targets.to_vec();
    if shuffle {
        // Simple shuffle using process id as seed
        let seed = std::process::id() as usize;
        for i in (1..targets.len()).rev() {
            let j = (seed + i * 31) % (i + 1);
            targets.swap(i, j);
        }
    }

    let mut locked: Vec<(i64, String, String)> = Vec::new();

    let (target_env, unlocked) =
        env::with_env(|v| (v.target.clone(), v.unlocked));

    let selflock_info: Option<(i64, state::File)> = if !target_env.is_empty() && !unlocked {
        let (startdir, pwd) = env::with_env(|v| (v.startdir.clone(), v.pwd.clone()));
        let me = format!("{}/{}/{}", startdir, pwd, target_env);
        let myfile = state::File::from_name(&me, true);
        let fid = myfile.id;
        Some((fid, myfile))
    } else {
        None
    };

    for t in &targets {
        if t.contains('\n') {
            logs::err(&format!(
                "{:?}: filenames containing newlines are not allowed.",
                t
            ));
            return 204;
        }
    }

    let cheat = || -> i32 {
        if selflock_info.is_none() {
            return 0;
        }
        let fid = selflock_info.as_ref().unwrap().0;
        let mut selflock = state::Lock::new(state::LOG_LOCK_MAGIC + fid);
        selflock.trylock();
        if !selflock.owned {
            1
        } else {
            selflock.unlock();
            0
        }
    };

    // First cycle: build without waiting for locks
    let mut seen = std::collections::HashSet::new();
    let keep_going = env::with_env(|v| v.keep_going);
    let is_unlocked = env::with_env(|v| v.unlocked);

    for t in &targets {
        if t.is_empty() {
            logs::err("cannot build the empty target (\"\").");
            retcode = 204;
            break;
        }
        assert!(state::is_flushed());
        if seen.contains(t) {
            continue;
        }
        seen.insert(t.clone());

        if !jobserver::has_token() {
            state::commit();
        }
        jobserver::ensure_token_or_cheat(t, &cheat);
        if retcode != 0 && !keep_going {
            break;
        }
        if !state::check_sane() {
            logs::err(".redo directory disappeared; cannot continue.");
            retcode = 205;
            break;
        }

        let f = state::File::from_name(t, true);
        let mut lock = state::Lock::new(f.id);
        if is_unlocked {
            lock.owned = true;
        } else {
            lock.trylock();
        }

        if !lock.owned {
            logs::meta("locked", &state::target_relpath(t));
            locked.push((f.id, t.clone(), f.name.clone()));
            std::mem::forget(lock); // don't drop/unlock
        } else {
            let mut f = state::File::from_name(t, true); // refresh
            build_job_start(t, &mut f, lock, shouldbuildfunc, &mut retcode);
        }
        state::commit();
        assert!(state::is_flushed());
    }

    // Second cycle: wait for locked targets
    while !locked.is_empty() || jobserver::running() {
        state::commit();
        jobserver::wait_all();
        jobserver::ensure_token_or_cheat("self", &cheat);

        if retcode != 0 && !keep_going {
            break;
        }

        if !locked.is_empty() {
            if !state::check_sane() {
                logs::err(".redo directory disappeared; cannot continue.");
                retcode = 205;
                break;
            }

            let (fid, t, _name) = locked.remove(0);
            let mut lock = state::Lock::new(fid);
            let mut backoff: f64 = 0.01;
            lock.trylock();
            while !lock.owned {
                let delay = (rand_float() * backoff.min(1.0)).max(0.001);
                std::thread::sleep(std::time::Duration::from_secs_f64(delay));
                backoff *= 2.0;
                logs::meta("waiting", &state::target_relpath(&t));
                match lock.trylock_returning_cycle_error() {
                    Err(_) => {
                        logs::err(&format!("cyclic dependency while building {}", nice(&t)));
                        retcode = 208;
                        return retcode;
                    }
                    Ok(_) => {}
                }
                if !lock.owned {
                    jobserver::release_mine();
                    lock.waitlock(false);
                    lock.unlock();
                    jobserver::ensure_token_or_cheat(&t, &cheat);
                    lock.trylock();
                }
            }
            assert!(lock.owned);
            logs::meta("unlocked", &state::target_relpath(&t));

            if state::File::from_name(&t, true).is_failed() {
                logs::err(&format!("{}: failed in another thread", nice(&t)));
                retcode = 2;
                lock.unlock();
            } else {
                let mut f = state::File::from_id(fid);
                build_job_start(&t, &mut f, lock, shouldbuildfunc, &mut retcode);
            }
        }
    }

    state::commit();
    retcode
}

fn rand_float() -> f64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (t as f64) / 4_294_967_296.0
}

#[derive(Debug)]
pub enum ShouldBuildResult {
    Clean,
    Dirty,
    NeedsBuild(Vec<state::File>),
}

fn build_job_start(
    t: &str,
    sf: &mut state::File,
    lock: state::Lock,
    shouldbuildfunc: &dyn Fn(&str) -> (bool, ShouldBuildResult),
    retcode: &mut i32,
) {
    // Check if we should build
    let (is_target, dirty) = shouldbuildfunc(t);

    match dirty {
        ShouldBuildResult::Clean => {
            if is_target {
                logs::meta("unchanged", &state::target_relpath(t));
            }
            drop(lock);
            return;
        }
        ShouldBuildResult::Dirty => {
            let no_oob = env::with_env(|v| v.no_oob);
            if no_oob {
                start_build(t, sf, lock, retcode);
            } else {
                start_build(t, sf, lock, retcode);
            }
        }
        ShouldBuildResult::NeedsBuild(deps) => {
            let no_oob = env::with_env(|v| v.no_oob);
            if no_oob {
                start_build(t, sf, lock, retcode);
            } else {
                start_deps_unlocked(t, sf, lock, &deps, retcode);
            }
        }
    }
}

fn start_build(
    t: &str,
    sf: &mut state::File,
    mut lock: state::Lock,
    retcode: &mut i32,
) {
    assert!(lock.owned);

    let newstamp = sf.read_stamp();
    if sf.is_generated
        && newstamp != state::STAMP_MISSING
        && (sf.is_override || state::detect_override(sf.stamp.as_deref().unwrap_or(""), &newstamp))
    {
        state::warn_override(&nice(t));
        if !sf.is_override {
            logs::warn(&format!("{} - old: {:?}", nice(t), sf.stamp));
            logs::warn(&format!("{} - new: {:?}", nice(t), newstamp));
            sf.set_override();
        }
        sf.save();
    }

    let t_exists = std::path::Path::new(t).exists();
    let t_is_dir = std::path::Path::new(&format!("{}/.", t)).is_dir();
    if t_exists && !t_is_dir && (sf.is_override || !sf.is_generated) {
        logs::debug2(&format!("-- static ({:?})", t));
        if !sf.is_override {
            sf.set_static();
        }
        sf.save();
        lock.unlock();
        return;
    }

    sf.zap_deps1();
    let (dodir, dofile, _basedir, basename, ext) = paths::find_do_file(sf);

    if dofile.is_none() {
        if std::path::Path::new(t).exists() {
            sf.set_static();
            sf.save();
            lock.unlock();
            return;
        } else {
            logs::err(&format!("no rule to redo {:?}", t));
            sf.set_failed();
            sf.save();
            lock.unlock();
            *retcode = 1;
            return;
        }
    }

    let dodir = dodir.unwrap();
    let dofile = dofile.unwrap();
    let basename = basename.unwrap();
    let ext = ext.unwrap();

    let tmpbase = format!("{}/{}{}", dodir, basename, ext);
    let tmpname = format!("{}.redo.tmp", tmpbase);
    helpers::unlink(&tmpname);

    // Create anonymous temp file for stdout capture
    let ffd = {
        let template = "/tmp/redo.XXXXXX";
        let (fd, path) = nix::unistd::mkstemp(template).expect("mkstemp failed");
        // Unlink immediately - we keep the fd open
        let _ = std::fs::remove_file(&path);
        fd.into_raw_fd()
    };
    helpers::close_on_exec(ffd, true);

    // Build argv
    let arg1 = format!("{}{}", basename, ext);  // $1: target name with extension
    let arg2 = basename.clone();                 // $2: target name without extension
    // Like Python's os.path.abspath(self.tmpname)
    let abs_tmpname = if std::path::Path::new(&tmpname).is_absolute() {
        tmpname.clone()
    } else {
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        format!("{}/{}", cwd, tmpname)
    };
    let tmpname_rel = state::relpath(&abs_tmpname, &dodir);

    let mut argv = vec![
        "sh".to_string(),
        "-e".to_string(),
        dofile.clone(),
        arg1.clone(),
        arg2.clone(),
        tmpname_rel,
    ];

    let verbose = env::with_env(|v| v.verbose);
    let xtrace = env::with_env(|v| v.xtrace);
    if verbose != 0 {
        argv[1].push('v');
    }
    if xtrace != 0 {
        argv[1].push('x');
    }

    // Check for shebang
    let dopath = format!("{}/{}", dodir, dofile);
    if let Ok(mut file) = std::fs::File::open(&dopath) {
        let mut first_line = String::new();
        let mut reader = std::io::BufReader::new(&mut file);
        let _ = std::io::BufRead::read_line(&mut reader, &mut first_line);
        let first_line = first_line.trim();
        if first_line.starts_with("#!/") {
            let parts: Vec<String> = first_line[2..]
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            argv.splice(0..2, parts);
        }
    }

    // Create log file
    let log_enabled = env::with_env(|v| v.log);
    if log_enabled != 0 {
        let lfend = state::logname(sf.id);
        let lfdir = std::path::Path::new(&lfend)
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();
        // Create temp log and rename atomically
        let tmplog = format!("{}/redo.log.tmp.{}", lfdir, std::process::id());
        let _ = std::fs::File::create(&tmplog);
        let _ = std::fs::rename(&tmplog, &lfend);
    }

    // Record .do file as static source
    let mut dof = state::File::from_name(&dopath, true);
    dof.set_static();
    dof.save();
    state::commit();

    logs::meta("do", &state::target_relpath(t));

    let before_t = try_stat(t);
    let t_owned = t.to_string();
    let sf_id = sf.id;
    let sf_clone = sf.clone();
    let tmpname_owned = tmpname.clone();
    let retcode_ptr = SendPtr(retcode as *mut i32);
    let lock_fid = lock.fid;

    // Transfer lock ownership to the job
    std::mem::forget(lock);

    let dodir_owned = dodir.clone();
    let argv_owned = argv.clone();
    let depth = env::with_env(|v| v.depth.clone());

    let jobfunc = Box::new(move || {
        // Child process
        assert!(state::is_flushed());
        let newp = std::fs::canonicalize(&dodir_owned)
            .unwrap_or_else(|_| std::path::PathBuf::from(&dodir_owned))
            .to_string_lossy()
            .to_string();
        let startdir = env::with_env(|v| v.startdir.clone());
        std::env::remove_var("CDPATH");
        std::env::set_var("REDO_PWD", state::relpath(&newp, &startdir));
        std::env::set_var("REDO_TARGET", &arg1);
        std::env::set_var("REDO_DEPTH", format!("{}  ", depth));

        let xtrace_val = env::with_env(|v| v.xtrace);
        let verbose_val = env::with_env(|v| v.verbose);
        if xtrace_val == 1 {
            std::env::set_var("REDO_XTRACE", "0");
        }
        if verbose_val == 1 {
            std::env::set_var("REDO_VERBOSE", "0");
        }

        cycles::add(lock_fid);

        if !dodir_owned.is_empty() {
            let _ = std::env::set_current_dir(&dodir_owned);
        }

        dup2(ffd, 1).expect("dup2 failed");
        drop(unsafe { OwnedFd::from_raw_fd(ffd) });
        helpers::close_on_exec(1, false);

        let log_val = env::with_env(|v| v.log);
        if log_val != 0 {
            let log_inode = env::with_env(|v| v.log_inode.clone());
            let cur_inode = match fstat(2) {
                Ok(st) => st.st_ino.to_string(),
                Err(_) => String::new(),
            };
            if log_inode.is_empty() || cur_inode == log_inode {
                let logname = state::logname(sf_id);
                if let Ok(logf) = std::fs::File::create(&logname) {
                    let logfd = logf.as_raw_fd();
                    let new_inode = match fstat(logfd) {
                        Ok(st) => st.st_ino.to_string(),
                        Err(_) => String::new(),
                    };
                    std::env::set_var("REDO_LOG", "1");
                    std::env::set_var("REDO_LOG_INODE", &new_inode);
                    dup2(logfd, 2).expect("dup2 failed");
                    helpers::close_on_exec(2, false);
                }
            }
        } else {
            std::env::remove_var("REDO_LOG_INODE");
            std::env::set_var("REDO_LOG", "");
        }

        // Reset SIGPIPE to default
        unsafe {
            signal(Signal::SIGPIPE, SigHandler::SigDfl).ok();
        }

        let c_argv: Vec<std::ffi::CString> = argv_owned
            .iter()
            .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
            .collect();
        let _ = nix::unistd::execvp(&c_argv[0], &c_argv);
        eprintln!("redo: exec {:?} failed", argv_owned);
        std::process::exit(201);
    });

    let donefunc = Box::new(move |t: &str, rv: i32| {
        // Force capture of retcode_ptr as whole SendPtr (not partial field capture)
        let _ = &retcode_ptr;
        let rv = record_new_state(t, rv, &sf_clone, ffd, &tmpname_owned, before_t.as_ref());
        state::commit();
        // Unlock
        let lockfile = *state::LOCKFILE_FD.lock().unwrap();
        do_unlock(lockfile, lock_fid);
        // Update retcode
        unsafe {
            if rv != 0 {
                *retcode_ptr.0 = 1;
            }
        }
    });

    jobserver::start(&t_owned, jobfunc, donefunc);
}

fn do_unlock(lockfile: i32, fid: i64) {
    let fl = nix::libc::flock {
        l_type: nix::libc::F_UNLCK as i16,
        l_whence: nix::libc::SEEK_SET as i16,
        l_start: fid as nix::libc::off_t,
        l_len: 1,
        l_pid: 0,
    };
    let _ = fcntl(lockfile, FcntlArg::F_SETLK(&fl));
}

fn record_new_state(
    t: &str,
    mut rv: i32,
    sf: &state::File,
    ffd: i32,
    tmpname: &str,
    before_t: Option<&std::fs::Metadata>,
) -> i32 {
    state::check_sane();
    let after_t = try_stat(t);

    let st1_size = match fstat(ffd) {
        Ok(st) => st.st_size as u64,
        Err(_) => 0,
    };
    let st2 = try_stat(tmpname);

    use std::os::unix::fs::MetadataExt;

    if let Some(after) = &after_t {
        let before_mtime = before_t.map(|b| b.mtime());
        let after_mtime = after.mtime();
        if (before_t.is_none() || before_mtime != Some(after_mtime)) && !after.is_dir() {
            logs::err(&format!("{} modified {} directly!", "dofile", t));
            logs::err("...you should update $3 (a temp file) or stdout, not $1.");
            rv = 206;
        }
    }
    if st2.is_some() && st1_size > 0 {
        logs::err(&format!("{} wrote to stdout *and* created $3.", "dofile"));
        logs::err("...you should write status messages to stderr, not stdout.");
        rv = 207;
    }

    if rv == 0 {
        if st1_size > 0 && st2.is_none() {
            // Script wrote to stdout - copy to tmpname
            helpers::unlink(tmpname);
            match std::fs::File::create(tmpname) {
                Ok(mut newf) => {
                    lseek(ffd, 0, Whence::SeekSet).expect("lseek failed");
                    let mut buf = [0u8; 1024 * 1024];
                    loop {
                        let n = nix::unistd::read(ffd, &mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        let _ = newf.write_all(&buf[..n]);
                    }
                }
                Err(e) => {
                    let dnt = std::path::Path::new(t)
                        .parent()
                        .map(|p| {
                            std::fs::canonicalize(p)
                                .unwrap_or(p.to_path_buf())
                                .to_string_lossy()
                                .to_string()
                        })
                        .unwrap_or_default();
                    if !std::path::Path::new(&dnt).exists() {
                        logs::err(&format!("{}: target dir {:?} does not exist!", t, dnt));
                    } else {
                        logs::err(&format!("{}: copy stdout: {}", t, e));
                    }
                    rv = 209;
                }
            }
        }
        if rv == 0 {
            let st2 = try_stat(tmpname);
            if st2.is_some() {
                if let Err(e) = std::fs::rename(tmpname, t) {
                    logs::err(&format!("{}: rename {}: {}", t, tmpname, e));
                    rv = 209;
                }
            } else {
                helpers::unlink(t);
            }
        }
        if rv == 0 {
            let mut sf = sf.clone();
            sf.refresh();
            sf.is_generated = true;
            sf.is_override = false;
            if sf.is_checked() || sf.is_changed() {
                sf.stamp = Some(sf.read_stamp());
            } else {
                sf.csum = None;
                sf.update_stamp(false);
                sf.set_changed();
            }
            sf.zap_deps2();
            sf.save();
        }
    }
    if rv != 0 {
        helpers::unlink(tmpname);
        let mut sf = sf.clone();
        sf.set_failed();
        sf.zap_deps2();
        sf.save();
    }

    drop(unsafe { OwnedFd::from_raw_fd(ffd) });
    logs::meta("done", &format!("{} {}", rv, state::target_relpath(t)));
    rv
}

fn start_deps_unlocked(
    t: &str,
    sf: &state::File,
    lock: state::Lock,
    dirty_deps: &[state::File],
    retcode: &mut i32,
) {
    let here = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let base = env::with_env(|v| v.base.clone());

    let fix = |p: &str| -> String {
        state::relpath(&format!("{}/{}", base, p), &here)
    };

    let mut argv = vec!["redo-unlocked".to_string(), fix(&sf.name)];
    let mut seen = std::collections::HashSet::new();
    for d in dirty_deps {
        let fixed = fix(&d.name);
        if seen.insert(fixed.clone()) {
            argv.push(fixed);
        }
    }

    logs::meta("check", &state::target_relpath(t));
    state::commit();

    let depth = env::with_env(|v| v.depth.clone());
    let t_owned = t.to_string();
    let lock_fid = lock.fid;
    std::mem::forget(lock);
    let retcode_ptr = SendPtr(retcode as *mut i32);

    let jobfunc = Box::new(move || {
        std::env::set_var("REDO_DEPTH", format!("{}  ", depth));
        unsafe {
            signal(Signal::SIGPIPE, SigHandler::SigDfl).ok();
        }
        let c_argv: Vec<std::ffi::CString> = argv
            .iter()
            .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
            .collect();
        let _ = nix::unistd::execvp(&c_argv[0], &c_argv);
        std::process::exit(201);
    });

    let donefunc = Box::new(move |_t: &str, rv: i32| {
        // Force capture of retcode_ptr as whole SendPtr (not partial field capture)
        let _ = &retcode_ptr;
        let lockfile = *state::LOCKFILE_FD.lock().unwrap();
        do_unlock(lockfile, lock_fid);
        unsafe {
            if rv != 0 {
                *retcode_ptr.0 = 1;
            }
        }
    });

    jobserver::start(&t_owned, jobfunc, donefunc);
}
