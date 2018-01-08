// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomons` 0.1.0
#![feature(duration_extras, match_default_bindings, try_from)]
#![deny(missing_docs)]

#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate getset;
#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_try;

extern crate bincode;
extern crate clap;
extern crate futures;
extern crate git2;
extern crate rand;
extern crate repomon;
extern crate slog_async;
extern crate slog_term;
extern crate tokio_core;
extern crate tokio_io;
extern crate uuid;

mod branch;
mod callbacks;
mod error;
mod log;
mod repo;
mod run;

use std::io::{self, Write};
use std::process;

/// CLI Entry Point
fn main() {
    match run::run() {
        Ok(i) => process::exit(i),
        Err(e) => {
            writeln!(io::stderr(), "{}", e).expect("Unable to write to stderr!");
            process::exit(1)
        }
    }
}
