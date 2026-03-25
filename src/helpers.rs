// helpers.rs - Utility functions
//
// Based on redo/helpers.py from apenwarr/redo
// Copyright 2010-2018 Avery Pennarun and contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0

use std::io;

use nix::fcntl::{fcntl, FcntlArg, FdFlag};

/// Exception-like mechanism for immediate return with a specific exit code.
#[derive(Debug)]
pub struct ImmediateReturn {
    pub rv: i32,
}

impl ImmediateReturn {
    pub fn new(rv: i32) -> Self {
        ImmediateReturn { rv }
    }
}

impl std::fmt::Display for ImmediateReturn {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "immediate return with exit code {}", self.rv)
    }
}

impl std::error::Error for ImmediateReturn {}

/// Delete a file if it exists. Does not error if it doesn't exist.
pub fn unlink(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            eprintln!("redo: unlink {}: {}", path, e);
        }
    }
}

/// Set or clear the close-on-exec flag for a file descriptor.
pub fn close_on_exec(fd: i32, yes: bool) {
    let flags = match fcntl(fd, FcntlArg::F_GETFD) {
        Ok(flags) => FdFlag::from_bits_truncate(flags),
        Err(_) => return,
    };
    let new_flags = if yes {
        flags | FdFlag::FD_CLOEXEC
    } else {
        flags & !FdFlag::FD_CLOEXEC
    };
    let _ = fcntl(fd, FcntlArg::F_SETFD(new_flags));
}

/// Check if a file descriptor is valid.
pub fn fd_exists(fd: i32) -> bool {
    fcntl(fd, FcntlArg::F_GETFD).is_ok()
}

/// Convert a string to an integer, returning 0 on error (C's atoi semantics).
pub fn atoi(v: &str) -> i64 {
    v.trim().parse::<i64>().unwrap_or(0)
}
