// jobserver.rs - GNU make-compatible jobserver
//
// Based on redo/jobserver.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{helpers, logs, state};
use std::collections::HashMap;
use std::os::unix::io::{BorrowedFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Mutex;

use nix::fcntl::{fcntl, FcntlArg};
use nix::sys::select::FdSet;
use nix::sys::time::TimeVal;
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::{fork, pipe, ForkResult, Pid};

static TOPLEVEL: Mutex<i32> = Mutex::new(0);
static MYTOKENS: Mutex<i32> = Mutex::new(1);
static CHEATS: Mutex<i32> = Mutex::new(0);
static TOKENFDS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static CHEATFDS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static WAITFDS: Mutex<Option<HashMap<i32, Job>>> = Mutex::new(None);

fn get_waitfds() -> std::sync::MutexGuard<'static, Option<HashMap<i32, Job>>> {
    let mut guard = WAITFDS.lock().unwrap();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

fn create_tokens(n: i32) {
    let mut mytokens = MYTOKENS.lock().unwrap();
    let mut cheats = CHEATS.lock().unwrap();
    assert!(n >= 0);
    assert!(*cheats >= 0);
    for _ in 0..n {
        if *cheats > 0 {
            *cheats -= 1;
        } else {
            *mytokens += 1;
        }
    }
}

fn destroy_tokens(n: i32) {
    let mut mytokens = MYTOKENS.lock().unwrap();
    assert!(*mytokens >= n);
    *mytokens -= n;
}

fn release(n: i32) {
    let mut mytokens = MYTOKENS.lock().unwrap();
    let mut cheats = CHEATS.lock().unwrap();
    assert!(n >= 0);
    assert!(*mytokens >= n);
    let mut n_to_share = 0;
    for _ in 0..n {
        *mytokens -= 1;
        if *cheats > 0 {
            *cheats -= 1;
        } else {
            n_to_share += 1;
        }
    }
    assert!(*mytokens >= 0);
    assert!(*cheats >= 0);
    if n_to_share > 0 {
        let tokenfds = TOKENFDS.lock().unwrap();
        if let Some((_, w)) = *tokenfds {
            let buf = vec![b't'; n_to_share as usize];
            let _ = nix::unistd::write(unsafe { BorrowedFd::borrow_raw(w) }, &buf);
        }
    }
}

fn release_except_mine() {
    let mytokens = *MYTOKENS.lock().unwrap();
    assert!(mytokens > 0);
    release(mytokens - 1);
}

pub fn release_mine() {
    let mytokens = *MYTOKENS.lock().unwrap();
    assert!(mytokens >= 1);
    release(1);
}

fn make_pipe(startfd: i32) -> (i32, i32) {
    let (read_end, write_end) = pipe().expect("pipe() failed");
    let a = read_end.into_raw_fd();
    let b = write_end.into_raw_fd();
    let fa = fcntl(a, FcntlArg::F_DUPFD(startfd)).expect("fcntl F_DUPFD failed");
    let fb = fcntl(b, FcntlArg::F_DUPFD(startfd + 1)).expect("fcntl F_DUPFD failed");
    drop(unsafe { OwnedFd::from_raw_fd(a) });
    drop(unsafe { OwnedFd::from_raw_fd(b) });
    (fa, fb)
}

fn try_read(fd: i32, _n: usize) -> Option<Vec<u8>> {
    assert!(state::is_flushed());

    // Non-blocking read using select with timeout 0
    {
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        let mut readfds = FdSet::new();
        readfds.insert(borrowed_fd);
        let mut timeout = TimeVal::new(0, 0);
        let rv = nix::sys::select::select(
            Some(fd + 1),
            Some(&mut readfds),
            None::<&mut FdSet>,
            None::<&mut FdSet>,
            Some(&mut timeout),
        );
        match rv {
            Ok(n) if n <= 0 => return None,
            Err(_) => return None,
            _ => {}
        }
    }

    // Socket is readable - try to read with alarm fallback
    let mut buf = [0u8; 1];
    match nix::unistd::read(fd, &mut buf) {
        Ok(0) => Some(vec![]), // EOF
        Ok(n) => Some(buf[..n].to_vec()),
        Err(_) => None,
    }
}

fn try_read_all(fd: i32, n: usize) -> Vec<u8> {
    let mut result = Vec::new();
    loop {
        match try_read(fd, n) {
            Some(b) if b.is_empty() => break,
            Some(b) => result.extend(b),
            None => break,
        }
    }
    result
}

pub fn setup(maxjobs: i32) {
    assert!(maxjobs >= 0);

    let mut tokenfds = TOKENFDS.lock().unwrap();
    assert!(tokenfds.is_none(), "jobserver already initialized");

    let flags = format!(" {} ", std::env::var("MAKEFLAGS").unwrap_or_default());

    // Try to find jobserver auth in MAKEFLAGS
    let mut found_fds: Option<(i32, i32)> = None;
    for find in ["--jobserver-auth=", "--jobserver-fds="] {
        if let Some(pos) = flags.find(find) {
            let rest = &flags[pos + find.len()..];
            if let Some(space) = rest.find(' ') {
                let arg = &rest[..space];
                let parts: Vec<&str> = arg.splitn(2, ',').collect();
                if parts.len() == 2 {
                    let a = helpers::atoi(parts[0]) as i32;
                    let b = helpers::atoi(parts[1]) as i32;
                    if a > 0 && b > 0 {
                        if !helpers::fd_exists(a) || !helpers::fd_exists(b) {
                            logs::err("broken --jobserver-auth from parent process:");
                            logs::err("  using GNU make? prefix your Makefile rule with \"+\"");
                            std::process::exit(200);
                        }
                        if maxjobs == 1 {
                            // serialize
                        } else if maxjobs > 1 {
                            logs::warn(&format!(
                                "warning: -j{} forced in sub-redo; starting new jobserver.",
                                maxjobs
                            ));
                        } else {
                            found_fds = Some((a, b));
                        }
                    }
                }
            }
            break;
        }
    }

    if let Some(fds) = found_fds {
        *tokenfds = Some(fds);
    }

    // Cheat fds
    let mut cheatfds = CHEATFDS.lock().unwrap();
    let cheats_env = if maxjobs == 0 {
        std::env::var("REDO_CHEATFDS").unwrap_or_default()
    } else {
        String::new()
    };
    *cheatfds = None;
    if !cheats_env.is_empty() {
        let parts: Vec<&str> = cheats_env.splitn(2, ',').collect();
        if parts.len() == 2 {
            let a = helpers::atoi(parts[0]) as i32;
            let b = helpers::atoi(parts[1]) as i32;
            if a > 0 && b > 0 && helpers::fd_exists(a) && helpers::fd_exists(b) {
                *cheatfds = Some((a, b));
            }
        }
    }
    if cheatfds.is_none() {
        *cheatfds = Some(make_pipe(102));
        let (a, b) = cheatfds.unwrap();
        std::env::set_var("REDO_CHEATFDS", format!("{},{}", a, b));
    }

    if tokenfds.is_none() {
        let realmax = if maxjobs > 0 { maxjobs } else { 1 };
        *TOPLEVEL.lock().unwrap() = realmax;
        let fds = make_pipe(100);
        *tokenfds = Some(fds);

        // Create and release extra tokens
        drop(tokenfds);
        create_tokens(realmax - 1);
        release_except_mine();
        let tokenfds = TOKENFDS.lock().unwrap();
        let (a, b) = tokenfds.unwrap();
        std::env::set_var(
            "MAKEFLAGS",
            format!(
                " -j --jobserver-auth={},{} --jobserver-fds={},{}",
                a, b, a, b
            ),
        );
    }
}

fn wait_internal(want_token: bool, max_delay: Option<f64>) {
    let tokenfds = TOKENFDS.lock().unwrap();
    let cheatfds_val = CHEATFDS.lock().unwrap().unwrap();

    let mut readfds = FdSet::new();
    let mut max_fd: i32 = 0;

    {
        let waitfds = get_waitfds();
        for &fd in waitfds.as_ref().unwrap().keys() {
            readfds.insert(unsafe { BorrowedFd::borrow_raw(fd) });
            if fd > max_fd {
                max_fd = fd;
            }
        }
    }

    if want_token {
        if let Some((r, _)) = *tokenfds {
            readfds.insert(unsafe { BorrowedFd::borrow_raw(r) });
            if r > max_fd {
                max_fd = r;
            }
        }
    }
    drop(tokenfds);

    assert!(state::is_flushed());

    let mut timeout_val = max_delay.map(|d| {
        let secs = d as i64;
        let usecs = ((d - secs as f64) * 1_000_000.0) as nix::sys::time::suseconds_t;
        TimeVal::new(secs, usecs)
    });

    let result = nix::sys::select::select(
        Some(max_fd + 1),
        Some(&mut readfds),
        None::<&mut FdSet>,
        None::<&mut FdSet>,
        timeout_val.as_mut(),
    );

    match result {
        Err(_) => return,
        Ok(_) => {}
    }

    let tokenfds = TOKENFDS.lock().unwrap();
    let token_r = tokenfds.map(|(r, _)| r);
    drop(tokenfds);

    // Process completed jobs
    let fds_ready: Vec<i32> = {
        let waitfds = get_waitfds();
        waitfds
            .as_ref()
            .unwrap()
            .keys()
            .filter(|&&fd| readfds.contains(unsafe { BorrowedFd::borrow_raw(fd) }))
            .cloned()
            .collect()
    };

    for fd in fds_ready {
        if Some(fd) == token_r {
            continue;
        }
        let pd = {
            let mut waitfds = get_waitfds();
            waitfds.as_mut().unwrap().remove(&fd)
        };
        if let Some(mut pd) = pd {
            // Check cheatfds for cheat tokens
            let b = try_read(cheatfds_val.0, 1);
            if b.is_some() && !b.as_ref().unwrap().is_empty() {
                // someone exited with cheats > 0, don't recreate token
            } else {
                create_tokens(1);
                if has_token() {
                    release_except_mine();
                }
            }

            drop(unsafe { OwnedFd::from_raw_fd(fd) });
            match waitpid(Pid::from_raw(pd.pid), None) {
                Ok(WaitStatus::Exited(_, code)) => {
                    pd.rv = code;
                }
                Ok(WaitStatus::Signaled(_, sig, _)) => {
                    pd.rv = -(sig as i32);
                }
                _ => {
                    pd.rv = 201;
                }
            }
            (pd.donefunc)(&pd.name, pd.rv);
        }
    }
}

pub fn has_token() -> bool {
    *MYTOKENS.lock().unwrap() >= 1
}

fn ensure_token(_reason: &str, max_delay: Option<f64>) {
    assert!(state::is_flushed());
    assert!(*MYTOKENS.lock().unwrap() <= 1);

    loop {
        if *MYTOKENS.lock().unwrap() >= 1 {
            break;
        }
        wait_internal(true, max_delay);
        if *MYTOKENS.lock().unwrap() >= 1 {
            break;
        }

        // Try to read a token
        let tokenfds = TOKENFDS.lock().unwrap();
        if let Some((r, _)) = *tokenfds {
            drop(tokenfds);
            if let Some(b) = try_read(r, 1) {
                if b.is_empty() {
                    panic!("unexpected EOF on token read");
                }
                *MYTOKENS.lock().unwrap() += 1;
                break;
            }
        } else {
            drop(tokenfds);
        }

        if max_delay.is_some() {
            break;
        }
    }
}

pub fn ensure_token_or_cheat(reason: &str, cheatfunc: &dyn Fn() -> i32) {
    let mut backoff: f64 = 0.01;
    while !has_token() {
        while running() && !has_token() {
            ensure_token(reason, None);
        }
        ensure_token(reason, Some(backoff.min(1.0)));
        backoff *= 2.0;
        if !has_token() {
            let n = cheatfunc();
            if n > 0 {
                *MYTOKENS.lock().unwrap() += n;
                *CHEATS.lock().unwrap() += n;
                break;
            }
        }
    }
}

pub fn running() -> bool {
    let waitfds = get_waitfds();
    !waitfds.as_ref().unwrap().is_empty()
}

pub fn wait_all() {
    assert!(state::is_flushed());
    loop {
        while *MYTOKENS.lock().unwrap() >= 2 {
            release(1);
        }
        if !running() {
            break;
        }
        if *MYTOKENS.lock().unwrap() >= 1 {
            release_mine();
        }
        wait_internal(false, None);
    }

    let toplevel = *TOPLEVEL.lock().unwrap();
    if toplevel > 0 {
        if *MYTOKENS.lock().unwrap() >= 1 {
            release_mine();
        }
        let tokenfds = TOKENFDS.lock().unwrap();
        let cheatfds_val = CHEATFDS.lock().unwrap().unwrap();
        if let Some((r, w)) = *tokenfds {
            let tokens = try_read_all(r, 8192);
            let cheats = try_read_all(cheatfds_val.0, 8192);
            if (tokens.len() as i32 - cheats.len() as i32) != toplevel {
                eprintln!(
                    "on exit: expected {} tokens; found {}-{}",
                    toplevel,
                    tokens.len(),
                    cheats.len()
                );
            }
            let _ = nix::unistd::write(unsafe { BorrowedFd::borrow_raw(w) }, &tokens);
        }
    }
}

pub fn force_return_tokens() {
    let n = {
        let waitfds = get_waitfds();
        let count = waitfds.as_ref().unwrap().len() as i32;
        count
    };

    {
        let mut waitfds = get_waitfds();
        waitfds.as_mut().unwrap().clear();
    }

    create_tokens(n);
    if has_token() {
        release_except_mine();
        assert!(*MYTOKENS.lock().unwrap() == 1);
    }

    let cheats = *CHEATS.lock().unwrap();
    let mytokens = *MYTOKENS.lock().unwrap();
    assert!(cheats <= mytokens);
    assert!(cheats == 0 || cheats == 1);

    if cheats > 0 {
        destroy_tokens(cheats);
        let cheatfds = CHEATFDS.lock().unwrap();
        if let Some((_, w)) = *cheatfds {
            let buf = vec![b't'; cheats as usize];
            let _ = nix::unistd::write(unsafe { BorrowedFd::borrow_raw(w) }, &buf);
        }
    }
    assert!(state::is_flushed());
}

pub struct Job {
    pub name: String,
    pub pid: i32,
    pub rv: i32,
    pub donefunc: Box<dyn Fn(&str, i32) + Send>,
}

pub fn start(
    reason: &str,
    jobfunc: Box<dyn FnOnce() + Send>,
    donefunc: Box<dyn Fn(&str, i32) + Send>,
) {
    assert!(state::is_flushed());
    assert!(*MYTOKENS.lock().unwrap() == 1);
    destroy_tokens(1);

    let (r, w) = make_pipe(50);

    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            drop(unsafe { OwnedFd::from_raw_fd(r) });
            jobfunc();
            // jobfunc should not return (it should call execvp)
            std::process::exit(201);
        }
        Ok(ForkResult::Parent { child }) => {
            helpers::close_on_exec(r, true);
            drop(unsafe { OwnedFd::from_raw_fd(w) });
            let job = Job {
                name: reason.to_string(),
                pid: child.as_raw(),
                rv: 0,
                donefunc,
            };
            let mut waitfds = get_waitfds();
            waitfds.as_mut().unwrap().insert(r, job);
        }
        Err(_) => panic!("fork failed"),
    }
}
