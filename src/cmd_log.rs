// cmd_log.rs - redo-log: display build logs
//
// Based on redo/cmd_log.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::{env, helpers, logs, options, state};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::os::unix::io::FromRawFd;

const OPTSPEC: &str = "\
redo-log [options...] [targets...]
--
r,recursive     show build logs for dependencies too
u,unchanged     show lines for dependencies not needing to be rebuilt
f,follow        keep watching for more lines to be appended (like tail -f)
no-details      only show 'redo' recursion trace, not build output
no-status       don't display build summary line in --follow
no-pretty       don't pretty-print logs, show raw @@REDO output instead
no-color        disable ANSI color; --color to force enable (default: auto)
debug-locks     print messages about file locking (useful for debugging)
debug-pids      print process ids in log messages (useful for debugging)
ack-fd=         (internal use only) print REDO-OK to this fd upon starting";

pub fn main() {
    let o = options::Options::new(OPTSPEC);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (opt, targets) = o.parse(&args);

    if targets.is_empty() {
        eprintln!("redo-log: give at least one target; maybe \"all\"?");
        std::process::exit(1);
    }

    state::init(&targets);

    let is_tty = nix::unistd::isatty(2).unwrap_or(false);
    let status_opt = opt.bool_val("status");
    let show_status = if opt.int_val("status") < 2 && !is_tty {
        false
    } else {
        status_opt
    };

    // redo-log sends output to stdout
    logs::setup(false, false, opt.int_val("pretty"), opt.int_val("color"));

    if opt.bool_val("debug_locks") {
        env::with_env_mut(|v| v.debug_locks = true);
    }
    if opt.bool_val("debug_pids") {
        env::with_env_mut(|v| v.debug_pids = true);
    }

    let ack_fd_str = opt.str_val("ack_fd");
    if !ack_fd_str.is_empty() {
        let ack_fd = helpers::atoi(&ack_fd_str) as i32;
        assert!(ack_fd > 2);
        use std::os::unix::io::BorrowedFd;
        let borrowed = unsafe { BorrowedFd::borrow_raw(ack_fd) };
        let written = nix::unistd::write(borrowed, b"REDO-OK\n").unwrap_or(0);
        if written != 8 {
            panic!("write to ack_fd returned wrong length");
        }
        drop(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(ack_fd) });
    }

    let topdir = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let mut already = HashSet::new();
    let mut depth: Vec<String> = Vec::new();
    let start_time = std::time::Instant::now();

    let recursive = opt.bool_val("recursive");
    let follow = opt.bool_val("follow");
    let details = opt.bool_val("details");
    let unchanged = opt.bool_val("unchanged");
    let debug_locks = opt.bool_val("debug_locks");

    for t in &targets {
        if t != "-" {
            logs::meta_with_pid("do", &rel(&topdir, ".", t), Some(0));
        }
        catlog(
            t,
            &topdir,
            &mut already,
            &mut depth,
            recursive,
            follow,
            details,
            unchanged,
            debug_locks,
            show_status,
            &start_time,
        );
    }
}

fn rel(top: &str, mydir: &str, path: &str) -> String {
    let full = format!("{}/{}/{}", top, mydir, path);
    let full_path = std::path::Path::new(&full);
    let top_path = std::path::Path::new(top);
    if let Ok(rel) = full_path.strip_prefix(top_path) {
        rel.to_string_lossy().to_string()
    } else {
        state::relpath(&full, top)
    }
}

fn is_locked(fid: Option<i64>) -> bool {
    if let Some(fid) = fid {
        let mut lock = state::Lock::new(fid);
        let got = lock.trylock();
        if got {
            lock.unlock();
            false
        } else {
            // Lock is held by someone else - but we created a Lock object
            // which asserted fid wasn't in use. Just return true.
            true
        }
    } else {
        false
    }
}

nix::ioctl_read_bad!(tiocgwinsz, nix::libc::TIOCGWINSZ, nix::libc::winsize);

fn tty_width() -> usize {
    let mut ws = nix::libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { tiocgwinsz(2, &mut ws) }.is_ok() && ws.ws_col > 0 {
        ws.ws_col as usize
    } else {
        std::env::var("WIDTH")
            .ok()
            .and_then(|w| w.parse().ok())
            .unwrap_or(70)
    }
}

#[allow(clippy::too_many_arguments)]
fn catlog(
    t: &str,
    topdir: &str,
    already: &mut HashSet<String>,
    depth: &mut Vec<String>,
    recursive: bool,
    follow: bool,
    details: bool,
    unchanged_opt: bool,
    debug_locks: bool,
    show_status: bool,
    start_time: &std::time::Instant,
) -> usize {
    let mut lines_written = 0;
    let mut interrupted = 0;

    if already.contains(t) {
        return 0;
    }
    if t != "-" {
        depth.push(t.to_string());
    }
    env::with_env_mut(|v| v.depth = "  ".repeat(depth.len()));
    already.insert(t.to_string());
    let mydir = std::path::Path::new(t)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let (fid, logname) = if t == "-" {
        (None, None)
    } else {
        match std::panic::catch_unwind(|| state::File::from_name(t, false)) {
            Ok(sf) => {
                let fid = sf.id;
                state::rollback();
                let logname = state::logname(fid);
                (Some(fid), Some(logname))
            }
            Err(_) => {
                eprintln!("redo-log: [{}] {:?}: not known to redo.", topdir, t);
                std::process::exit(24);
            }
        }
    };

    let mut delay: f64 = 0.01;
    let mut was_locked = is_locked(fid);
    let mut status: Option<String> = None;

    let reader: Box<dyn BufRead> = if t == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        match logname.as_ref().and_then(|n| std::fs::File::open(n).ok()) {
            Some(f) => Box::new(std::io::BufReader::new(f)),
            None => {
                // No log file yet
                if !follow || !was_locked {
                    if t != "-" {
                        depth.pop();
                    }
                    return 0;
                }
                // Wait and retry
                loop {
                    std::thread::sleep(std::time::Duration::from_secs_f64(delay.min(1.0)));
                    delay += 0.01;
                    was_locked = is_locked(fid);
                    if !follow || !was_locked {
                        if t != "-" {
                            depth.pop();
                        }
                        return 0;
                    }
                    if let Some(ref n) = logname {
                        if let Ok(f) = std::fs::File::open(n) {
                            break Box::new(std::io::BufReader::new(f));
                        }
                    }
                }
            }
        }
    };

    let mut reader = reader;
    let width = tty_width();
    let mut line_head = String::new();

    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).unwrap_or(0);

        if bytes_read == 0 {
            if !follow || !was_locked {
                break;
            }
            was_locked = is_locked(fid);
            if follow {
                if show_status && start_time.elapsed().as_secs_f64() > 1.0 {
                    // Display status line
                    let total_lines = lines_written;
                    let head = format!("redo {} ", total_lines);
                    let mut tail = String::new();
                    for n in depth.iter().rev() {
                        if n != "-" {
                            if tail.is_empty() {
                                tail = n.clone();
                            } else {
                                tail = format!("{} {}", n, tail);
                            }
                        }
                        if head.len() + tail.len() >= width {
                            break;
                        }
                    }
                    status = Some(format!("{}{}", head, tail));
                    if let Some(ref s) = status {
                        let _ = std::io::stdout().flush();
                        let truncated = if s.len() > width { &s[..width] } else { s };
                        let _ = write!(std::io::stderr(), "\r{:<width$}\r", truncated, width = width);
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs_f64(delay.min(1.0)));
                delay += 0.01;
            }
            continue;
        }

        delay = 0.01;
        if !line.ends_with('\n') {
            line_head.push_str(&line);
            continue;
        }
        if !line_head.is_empty() {
            line = format!("{}{}", line_head, line);
            line_head.clear();
        }

        if status.is_some() {
            let _ = std::io::stdout().flush();
            let _ = write!(std::io::stderr(), "\r{:<width$}\r", "", width = width);
            status = None;
        }

        // Parse @@REDO:...@@ lines
        if let Some(caps) = parse_redo_line(&line) {
            let kind = caps.0;
            let pid = caps.1;
            let text = caps.2;
            let relname = rel(topdir, &mydir, text);
            let fixname = {
                let p = format!("{}/{}", mydir, text);
                normalize_path(&p)
            };

            match kind {
                "unchanged" => {
                    if unchanged_opt {
                        if debug_locks {
                            logs::meta_with_pid(kind, &relname, Some(pid));
                        } else if !already.contains(&fixname) {
                            logs::meta_with_pid("do", &relname, Some(pid));
                        }
                        if recursive {
                            let sub_t = if mydir.is_empty() {
                                text.to_string()
                            } else {
                                format!("{}/{}", mydir, text)
                            };
                            let got = catlog(
                                &sub_t, topdir, already, depth, recursive, follow, details,
                                unchanged_opt, debug_locks, show_status, start_time,
                            );
                            interrupted += got;
                            lines_written += got;
                        }
                    }
                    already.insert(fixname);
                }
                "do" | "waiting" | "locked" | "unlocked" => {
                    if debug_locks {
                        logs::meta_with_pid(kind, &relname, Some(pid));
                        logs::log_write(line.trim_end());
                        interrupted += 1;
                        lines_written += 1;
                    } else if !already.contains(&fixname) {
                        logs::meta_with_pid("do", &relname, Some(pid));
                        interrupted += 1;
                        lines_written += 1;
                    }
                    if recursive {
                        let sub_t = if mydir.is_empty() {
                                text.to_string()
                            } else {
                                format!("{}/{}", mydir, text)
                            };
                        let got = catlog(
                            &sub_t, topdir, already, depth, recursive, follow, details,
                            unchanged_opt, debug_locks, show_status, start_time,
                        );
                        interrupted += got;
                        lines_written += got;
                    }
                    already.insert(fixname);
                }
                "done" => {
                    if let Some(space) = text.find(' ') {
                        let rv = &text[..space];
                        let name = &text[space + 1..];
                        let relname_done = rel(topdir, &mydir, name);
                        logs::meta_with_pid(kind, &format!("{} {}", rv, relname_done), Some(pid));
                        lines_written += 1;
                    }
                }
                _ => {
                    logs::log_write(line.trim_end());
                    lines_written += 1;
                }
            }
        } else {
            if details {
                if interrupted > 0 {
                    let d = env::with_env(|v| v.depth.clone());
                    env::with_env_mut(|v| {
                        if v.depth.len() >= 2 {
                            v.depth = v.depth[..v.depth.len() - 2].to_string();
                        }
                    });
                    logs::meta("resumed", t);
                    env::with_env_mut(|v| v.depth = d);
                    interrupted = 0;
                }
                logs::log_write(line.trim_end());
                lines_written += 1;
            }
        }
    }

    if status.is_some() {
        let _ = std::io::stdout().flush();
        let _ = write!(std::io::stderr(), "\r{:<width$}\r", "", width = width);
    }

    if !line_head.is_empty() {
        println!("{}", line_head);
    }

    if t != "-" {
        assert_eq!(depth.last().map(|s| s.as_str()), Some(t));
        depth.pop();
    }
    env::with_env_mut(|v| v.depth = "  ".repeat(depth.len()));
    lines_written
}

fn parse_redo_line(line: &str) -> Option<(&str, i32, &str)> {
    // Match @@REDO:kind:pid:time@@ text\n
    let start = line.find("@@REDO:")?;
    let rest = &line[start + 7..];
    let end = rest.find("@@ ")?;
    let words = &rest[..end];
    let text = rest[end + 3..].trim_end_matches('\n');

    let parts: Vec<&str> = words.splitn(3, ':').collect();
    if parts.len() < 3 {
        return None;
    }
    let kind = parts[0];
    let pid: i32 = parts[1].parse().unwrap_or(0);
    Some((kind, pid, text))
}

fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}
