// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomons` callbacks
use git2::{self, Cred, CredentialType, Progress, RemoteCallbacks};
use std::io::{self, Write};

/// Check credentials for connecting to remote.
fn check_creds(
    _url: &str,
    username: Option<&str>,
    cred_type: CredentialType,
) -> ::std::result::Result<Cred, git2::Error> {
    if cred_type.contains(git2::SSH_KEY) {
        Cred::ssh_key_from_agent(username.unwrap_or(""))
    } else {
        Err(git2::Error::from_str("Unable to authenticate"))
    }
}

/// Progress remote callback.
#[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
fn progress(progress: Progress) -> bool {
    writeln!(io::stdout(), "{}", progress.received_objects()).unwrap_or(());
    true
}

/// Side band remote callback.
fn side_band(text: &[u8]) -> bool {
    writeln!(io::stdout(), "{}", String::from_utf8_lossy(text)).unwrap_or(());
    true
}

pub fn get_default<'a>() -> RemoteCallbacks<'a> {
    let mut rcb = RemoteCallbacks::new();
    rcb.transfer_progress(progress);
    rcb.sideband_progress(side_band);
    rcb.credentials(check_creds);
    rcb
}
