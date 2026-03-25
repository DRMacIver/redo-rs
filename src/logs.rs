// logs.rs - Structured logging for redo operations
//
// Based on redo/logs.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use crate::env;
use std::io::Write;
use std::sync::Mutex;
use std::time::SystemTime;

static RED: Mutex<&str> = Mutex::new("");
static GREEN: Mutex<&str> = Mutex::new("");
static YELLOW: Mutex<&str> = Mutex::new("");
static BOLD: Mutex<&str> = Mutex::new("");
static PLAIN: Mutex<&str> = Mutex::new("");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogMode {
    Raw,
    Pretty,
}

static LOG_MODE: Mutex<LogMode> = Mutex::new(LogMode::Raw);
static LOG_TO_STDOUT: Mutex<bool> = Mutex::new(false);

fn check_tty(is_tty: bool, color: i64) {
    let term = std::env::var("TERM").unwrap_or_else(|_| "dumb".to_string());
    let color_ok = is_tty && term != "dumb";
    if (color != 0 && color_ok) || color >= 2 {
        *RED.lock().unwrap() = "\x1b[31m";
        *GREEN.lock().unwrap() = "\x1b[32m";
        *YELLOW.lock().unwrap() = "\x1b[33m";
        *BOLD.lock().unwrap() = "\x1b[1m";
        *PLAIN.lock().unwrap() = "\x1b[m";
    } else {
        *RED.lock().unwrap() = "";
        *GREEN.lock().unwrap() = "";
        *YELLOW.lock().unwrap() = "";
        *BOLD.lock().unwrap() = "";
        *PLAIN.lock().unwrap() = "";
    }
}

pub fn setup(tty_is_stderr: bool, parent_logs: bool, pretty: i64, color: i64) {
    if pretty != 0 && !parent_logs {
        let is_tty = if tty_is_stderr {
            nix::unistd::isatty(2).unwrap_or(false)
        } else {
            nix::unistd::isatty(1).unwrap_or(false)
        };
        check_tty(is_tty, color);
        *LOG_MODE.lock().unwrap() = LogMode::Pretty;
    } else {
        *LOG_MODE.lock().unwrap() = LogMode::Raw;
    }
    *LOG_TO_STDOUT.lock().unwrap() = !tty_is_stderr;
}

fn write_log(s: &str) {
    let mode = *LOG_MODE.lock().unwrap();
    let to_stdout = *LOG_TO_STDOUT.lock().unwrap();
    match mode {
        LogMode::Raw => {
            if to_stdout {
                let _ = writeln!(std::io::stdout(), "{}", s);
                let _ = std::io::stdout().flush();
            } else {
                let _ = writeln!(std::io::stderr(), "{}", s);
                let _ = std::io::stderr().flush();
            }
        }
        LogMode::Pretty => {
            pretty_write(s, to_stdout);
        }
    }
}

fn pretty_write(s: &str, to_stdout: bool) {
    let re_str = "@@REDO:([^@]+)@@ (.*)$";
    let re = regex_lite(re_str);
    let mut out: Box<dyn Write> = if to_stdout {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::io::stderr())
    };

    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();

    if let Some(caps) = re.captures(s) {
        let full_match = caps.get(0).unwrap();
        let prefix = &s[..s.len() - full_match.as_str().len()];
        if !prefix.is_empty() {
            let _ = write!(out, "{}", prefix);
        }
        let words_str = caps.get(1).unwrap().as_str();
        let text = caps.get(2).unwrap().as_str();
        let parts: Vec<&str> = words_str.splitn(3, ':').collect();
        if parts.len() < 3 {
            let _ = writeln!(out, "{}", s);
            let _ = out.flush();
            return;
        }
        let kind = parts[0];
        let pid: i32 = parts[1].parse().unwrap_or(0);

        let red = *RED.lock().unwrap();
        let green = *GREEN.lock().unwrap();
        let yellow = *YELLOW.lock().unwrap();
        let bold = *BOLD.lock().unwrap();
        let plain = *PLAIN.lock().unwrap();

        let (depth, debug, verbose, xtrace, log_val, debug_locks, debug_pids) =
            env::with_env(|v| {
                (
                    v.depth.clone(),
                    v.debug,
                    v.verbose,
                    v.xtrace,
                    v.log,
                    v.debug_locks,
                    v.debug_pids,
                )
            });

        let pretty_line = |out: &mut dyn Write, pid: i32, color: &str, msg: &str| {
            let redo_prefix = if debug_pids {
                format!("{:<6} redo  ", pid)
            } else {
                "redo  ".to_string()
            };
            let _ = writeln!(
                out,
                "{}{}{}{}{}{}",
                color,
                redo_prefix,
                depth,
                if !color.is_empty() { bold } else { "" },
                msg,
                plain
            );
        };

        match kind {
            "unchanged" => {
                if log_val != 0 || debug != 0 {
                    pretty_line(&mut *out, pid, "", &format!("{} (unchanged)", text));
                }
            }
            "check" => {
                pretty_line(&mut *out, pid, green, &format!("({})", text));
            }
            "do" => {
                pretty_line(&mut *out, pid, green, text);
            }
            "done" => {
                if let Some(space_pos) = text.find(' ') {
                    let rv: i32 = text[..space_pos].parse().unwrap_or(0);
                    let name = &text[space_pos + 1..];
                    if rv != 0 {
                        pretty_line(
                            &mut *out,
                            pid,
                            red,
                            &format!("{} (exit {})", name, rv),
                        );
                    } else if verbose != 0 || xtrace != 0 || debug != 0 {
                        pretty_line(&mut *out, pid, green, &format!("{} (done)", name));
                        let _ = writeln!(out);
                    }
                }
            }
            "resumed" => {
                pretty_line(&mut *out, pid, green, &format!("{} (resumed)", text));
            }
            "locked" => {
                if debug_locks {
                    pretty_line(&mut *out, pid, green, &format!("{} (locked...)", text));
                }
            }
            "waiting" => {
                if debug_locks {
                    pretty_line(&mut *out, pid, green, &format!("{} (WAITING)", text));
                }
            }
            "unlocked" => {
                if debug_locks {
                    pretty_line(
                        &mut *out,
                        pid,
                        green,
                        &format!("{} (...unlocked!)", text),
                    );
                }
            }
            "error" => {
                let _ = writeln!(out, "{}redo: {}{}{}", red, bold, text, plain);
            }
            "warning" => {
                let _ = writeln!(out, "{}redo: {}{}{}", yellow, bold, text, plain);
            }
            "debug" => {
                pretty_line(&mut *out, pid, "", text);
            }
            _ => {
                let _ = writeln!(out, "{}", s);
            }
        }
    } else {
        let _ = writeln!(out, "{}", s);
    }
    let _ = out.flush();
}

struct SimpleRegex {
    pattern: String,
}

struct SimpleCaptures<'a> {
    full: &'a str,
    groups: Vec<&'a str>,
}

impl<'a> SimpleCaptures<'a> {
    fn get(&self, idx: usize) -> Option<SimpleMatch<'a>> {
        if idx == 0 {
            Some(SimpleMatch { text: self.full })
        } else if idx <= self.groups.len() {
            Some(SimpleMatch {
                text: self.groups[idx - 1],
            })
        } else {
            None
        }
    }
}

struct SimpleMatch<'a> {
    text: &'a str,
}

impl<'a> SimpleMatch<'a> {
    fn as_str(&self) -> &'a str {
        self.text
    }
}

fn regex_lite(pattern: &str) -> SimpleRegexMatcher {
    SimpleRegexMatcher {
        _pattern: pattern.to_string(),
    }
}

struct SimpleRegexMatcher {
    _pattern: String,
}

impl SimpleRegexMatcher {
    fn captures<'a>(&self, s: &'a str) -> Option<SimpleCaptures<'a>> {
        // Match @@REDO:([^@]+)@@ (.*)$
        if let Some(start) = s.find("@@REDO:") {
            let rest = &s[start + 7..];
            if let Some(end) = rest.find("@@ ") {
                let group1 = &rest[..end];
                let group2 = &rest[end + 3..];
                let full = &s[start..];
                return Some(SimpleCaptures {
                    full,
                    groups: vec![group1, group2],
                });
            }
        }
        None
    }
}

pub fn log_write(s: &str) {
    write_log(s);
}

pub fn meta(kind: &str, s: &str) {
    meta_with_pid(kind, s, None);
}

pub fn meta_with_pid(kind: &str, s: &str, pid: Option<i32>) {
    assert!(!kind.contains(':'));
    assert!(!kind.contains('@'));
    assert!(!s.contains('\n'));
    let pid = pid.unwrap_or_else(|| nix::unistd::getpid().as_raw());
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    write_log(&format!("@@REDO:{}:{}:{:.4}@@ {}", kind, pid, now, s));
}

pub fn err(s: &str) {
    let s = s.trim_end();
    meta("error", s);
}

pub fn warn(s: &str) {
    let s = s.trim_end();
    meta("warning", s);
}

pub fn debug(s: &str) {
    let debug_level = env::with_env(|v| v.debug);
    if debug_level >= 1 {
        let s = s.trim_end();
        meta("debug", s);
    }
}

pub fn debug2(s: &str) {
    let debug_level = env::with_env(|v| v.debug);
    if debug_level >= 2 {
        let s = s.trim_end();
        meta("debug", s);
    }
}

pub fn debug3(s: &str) {
    let debug_level = env::with_env(|v| v.debug);
    if debug_level >= 3 {
        let s = s.trim_end();
        meta("debug", s);
    }
}
