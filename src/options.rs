// options.rs - Command-line options parsing
//
// Based on redo/options.py from apenwarr/redo
// Copyright 2011 Avery Pennarun and options.py contributors.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//    1. Redistributions of source code must retain the above copyright
//       notice, this list of conditions and the following disclaimer.
//
//    2. Redistributions in binary form must reproduce the above copyright
//       notice, this list of conditions and the following disclaimer in
//       the documentation and/or other materials provided with the
//       distribution.
//
// THIS SOFTWARE IS PROVIDED BY AVERY PENNARUN ``AS IS'' AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
// PURPOSE ARE DISCLAIMED.

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct OptDict {
    opts: HashMap<String, OptValue>,
}

#[derive(Debug, Clone)]
pub enum OptValue {
    Bool(bool),
    Int(i64),
    Str(String),
    None,
}

impl OptValue {
    pub fn as_bool(&self) -> bool {
        match self {
            OptValue::Bool(b) => *b,
            OptValue::Int(i) => *i != 0,
            OptValue::Str(s) => !s.is_empty(),
            OptValue::None => false,
        }
    }

    pub fn as_i64(&self) -> i64 {
        match self {
            OptValue::Bool(b) => *b as i64,
            OptValue::Int(i) => *i,
            OptValue::Str(s) => s.parse().unwrap_or(0),
            OptValue::None => 0,
        }
    }

    pub fn as_str(&self) -> String {
        match self {
            OptValue::Bool(b) => b.to_string(),
            OptValue::Int(i) => i.to_string(),
            OptValue::Str(s) => s.clone(),
            OptValue::None => String::new(),
        }
    }
}

impl OptDict {
    pub fn get(&self, key: &str) -> &OptValue {
        if let Some(k) = key.strip_prefix("no_").or_else(|| key.strip_prefix("no-")) {
            // Return negated
            static FALSE_VAL: OptValue = OptValue::Bool(false);
            static TRUE_VAL: OptValue = OptValue::Bool(true);
            match self.opts.get(k) {
                Some(OptValue::Bool(true)) => &FALSE_VAL,
                Some(OptValue::Bool(false)) => &TRUE_VAL,
                Some(OptValue::Int(0)) => &TRUE_VAL,
                Some(OptValue::Int(_)) => &FALSE_VAL,
                _ => &FALSE_VAL,
            }
        } else {
            self.opts.get(key).unwrap_or(&OptValue::None)
        }
    }

    pub fn set(&mut self, key: &str, val: OptValue) {
        let (actual_key, actual_val) =
            if let Some(k) = key.strip_prefix("no_").or_else(|| key.strip_prefix("no-")) {
                (
                    k.to_string(),
                    match val {
                        OptValue::Bool(b) => OptValue::Bool(!b),
                        OptValue::Int(i) => OptValue::Bool(i == 0),
                        _ => OptValue::Bool(false),
                    },
                )
            } else {
                (key.to_string(), val)
            };
        self.opts.insert(actual_key, actual_val);
    }

    pub fn bool_val(&self, key: &str) -> bool {
        self.get(key).as_bool()
    }

    pub fn int_val(&self, key: &str) -> i64 {
        self.get(key).as_i64()
    }

    pub fn str_val(&self, key: &str) -> String {
        self.get(key).as_str()
    }
}

#[derive(Debug, Clone)]
struct OptDef {
    long_name: String,
    short_name: Option<char>,
    has_param: bool,
    default: OptValue,
    negatable: bool,
}

pub struct Options {
    defs: Vec<OptDef>,
}

impl Options {
    pub fn new(optspec: &str) -> Self {
        let mut defs = Vec::new();
        let lines: Vec<&str> = optspec.lines().collect();
        let mut past_separator = false;

        for line in lines {
            let line = line.trim_start();
            if line == "--" {
                past_separator = true;
                continue;
            }
            if !past_separator || line.is_empty() || line.starts_with(' ') {
                continue;
            }

            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if parts.is_empty() {
                continue;
            }

            let mut flags_str = parts[0].to_string();
            let extra = if parts.len() > 1 {
                parts[1].trim()
            } else {
                ""
            };

            let has_param = flags_str.ends_with('=');
            if has_param {
                flags_str.pop();
            }

            // Parse default from [value]
            let default = if let Some(start) = extra.rfind('[') {
                if let Some(end) = extra[start..].find(']') {
                    let val = &extra[start + 1..start + end];
                    if let Ok(i) = val.parse::<i64>() {
                        OptValue::Int(i)
                    } else {
                        OptValue::Str(val.to_string())
                    }
                } else {
                    if has_param {
                        OptValue::None
                    } else {
                        OptValue::Bool(false)
                    }
                }
            } else if has_param {
                OptValue::None
            } else {
                OptValue::Bool(false)
            };

            let flag_names: Vec<&str> = flags_str.split(',').collect();
            let mut short = None;
            let mut long = String::new();

            for f in &flag_names {
                let f = f.trim();
                let f = if let Some(stripped) = f.strip_prefix("no-") {
                    stripped
                } else {
                    f
                };
                if f.len() == 1 {
                    short = Some(f.chars().next().unwrap());
                } else {
                    long = f.replace('-', "_");
                }
            }

            if long.is_empty() {
                if let Some(c) = short {
                    long = c.to_string();
                }
            }

            // Check if the original spec had a no- prefix
            let original_is_negated = flag_names.iter().any(|f| f.starts_with("no-"));

            defs.push(OptDef {
                long_name: long,
                short_name: short,
                has_param,
                default: if original_is_negated {
                    // If defined with no- prefix, the default is the positive form being true
                    match &default {
                        OptValue::Bool(b) => OptValue::Bool(!b),
                        _ => OptValue::Bool(true),
                    }
                } else {
                    default
                },
                negatable: !has_param,
            });
        }

        Options { defs }
    }

    pub fn parse(&self, args: &[String]) -> (OptDict, Vec<String>) {
        let mut opt = OptDict::default();

        // Set defaults
        for def in &self.defs {
            opt.set(&def.long_name, def.default.clone());
        }

        let mut extra = Vec::new();
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                extra.extend(args[i + 1..].iter().cloned());
                break;
            }

            if arg.starts_with("--") {
                let flag = &arg[2..];
                let (name, value) = if let Some(eq_pos) = flag.find('=') {
                    (&flag[..eq_pos], Some(flag[eq_pos + 1..].to_string()))
                } else {
                    (flag, None)
                };

                let is_negated = name.starts_with("no-");
                let actual_name = if is_negated { &name[3..] } else { name };
                let normalized = actual_name.replace('-', "_");

                if normalized == "help" || normalized == "usage" {
                    self.usage();
                    std::process::exit(97);
                }

                if let Some(def) = self.defs.iter().find(|d| d.long_name == normalized) {
                    if def.has_param {
                        let val = value.or_else(|| {
                            i += 1;
                            args.get(i).cloned()
                        });
                        if let Some(v) = val {
                            if let Ok(n) = v.parse::<i64>() {
                                opt.set(&def.long_name, OptValue::Int(n));
                            } else {
                                opt.set(&def.long_name, OptValue::Str(v));
                            }
                        }
                    } else if is_negated {
                        opt.set(&def.long_name, OptValue::Bool(false));
                    } else {
                        let cur = opt.get(&def.long_name).as_i64();
                        opt.set(&def.long_name, OptValue::Int(cur + 1));
                    }
                } else {
                    eprintln!("error: unknown option --{}", name);
                    std::process::exit(97);
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    let c = chars[j];
                    if c == 'h' || c == '?' {
                        self.usage();
                        std::process::exit(97);
                    }
                    if let Some(def) = self.defs.iter().find(|d| d.short_name == Some(c)) {
                        if def.has_param {
                            let val = if j + 1 < chars.len() {
                                Some(chars[j + 1..].iter().collect::<String>())
                            } else {
                                i += 1;
                                args.get(i).cloned()
                            };
                            if let Some(v) = val {
                                if let Ok(n) = v.parse::<i64>() {
                                    opt.set(&def.long_name, OptValue::Int(n));
                                } else {
                                    opt.set(&def.long_name, OptValue::Str(v));
                                }
                            }
                            break;
                        } else {
                            let cur = opt.get(&def.long_name).as_i64();
                            opt.set(&def.long_name, OptValue::Int(cur + 1));
                        }
                    } else {
                        eprintln!("error: unknown option -{}", c);
                        std::process::exit(97);
                    }
                    j += 1;
                }
            } else {
                extra.push(arg.clone());
            }

            i += 1;
        }

        (opt, extra)
    }

    fn usage(&self) {
        eprintln!("Options:");
        for def in &self.defs {
            let short = def
                .short_name
                .map(|c| format!("-{}, ", c))
                .unwrap_or_else(|| "    ".to_string());
            let param = if def.has_param { " ..." } else { "" };
            eprintln!("  {}--{}{}", short, def.long_name.replace('_', "-"), param);
        }
    }
}
