// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` slog logs.
use slog::{Drain, Level, LevelFilter, Logger};
use slog_async::Async;
use slog_term::{CompactFormat, TermDecorator};

/// slog `Logger`s for stdout and stderr.
#[derive(Clone, Debug, Getters)]
pub struct Logs {
    /// stdout logger.
    #[get = "pub"]
    stdout: Option<Logger>,
    /// stderr logger.
    #[get = "pub"]
    stderr: Option<Logger>,
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            stdout: None,
            stderr: Some(stderr_logger()),
        }
    }
}

impl Logs {
    /// Set the stdout filter level.
    pub fn set_stdout_level(&mut self, level: Level) -> &mut Self {
        self.stdout = Some(stdout_logger(level));
        self
    }
}

/// Setup the stderr slog `Logger`
fn stderr_logger() -> Logger {
    let stderr_decorator = TermDecorator::new().stderr().build();
    let stderr_drain = CompactFormat::new(stderr_decorator).build().fuse();
    let stderr_async_drain = Async::new(stderr_drain).build().fuse();
    let stderr_level_drain = LevelFilter::new(stderr_async_drain, Level::Error).fuse();
    Logger::root(
        stderr_level_drain,
        o!(
            "executable" => env!("CARGO_PKG_NAME"),
            "version" => env!("CARGO_PKG_VERSION")
        ),
    )
}

/// Setup the stdout slog `Logger`
fn stdout_logger(level: Level) -> Logger {
    let stdout_decorator = TermDecorator::new().stdout().build();
    let stdout_drain = CompactFormat::new(stdout_decorator).build().fuse();
    let stdout_async_drain = Async::new(stdout_drain).build().fuse();
    let stdout_level_drain = LevelFilter::new(stdout_async_drain, level).fuse();
    Logger::root(
        stdout_level_drain,
        o!(
            "executable" => env!("CARGO_PKG_NAME"),
            "version" => env!("CARGO_PKG_VERSION")
        ),
    )
}
