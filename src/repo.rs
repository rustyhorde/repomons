// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` repository operations.
use callbacks;
use error::Result;
use git2::{FetchOptions, Progress, Repository};
use git2::build::RepoBuilder;
use repomon::Remote;
use std::cell::RefCell;
use std::env;
use std::path::PathBuf;
use std::rc::Rc;
use term;

#[derive(Clone, Debug, Default, Getters, Setters)]
pub struct RepoConfig<'a> {
    #[get = "pub"]
    #[set = "pub"]
    basedir: PathBuf,
    #[get = "pub"]
    #[set = "pub"]
    repo: PathBuf,
    #[get = "pub"]
    #[set = "pub"]
    remotes: &'a [Remote],
}

#[derive(PartialEq)]
pub enum CloneState {
    Receiving,
    Resolving,
}

#[derive(Getters, MutGetters, Setters)]
pub struct CloneOutput {
    #[get_mut = "pub"] sideband: String,
    #[set = "pub"]
    #[get_mut = "pub"]
    progress: String,
    #[get = "pub"]
    #[set = "pub"]
    state: CloneState,
}

impl Default for CloneOutput {
    fn default() -> CloneOutput {
        CloneOutput {
            sideband: String::new(),
            progress: String::new(),
            state: CloneState::Receiving,
        }
    }
}

/// Discover the given repository at the given base directory, to try to clone it there.
pub fn discover_or_clone(config: &RepoConfig) -> Result<Repository> {
    env::set_current_dir(config.basedir())?;
    match Repository::discover(config.repo()) {
        Ok(repository) => Ok(repository),
        Err(_e) => {
            let origin: &Remote = config
                .remotes()
                .iter()
                .filter(|x| x.name() == "origin")
                .last()
                .ok_or("origin remote not found")?;
            let mut repo_builder = RepoBuilder::new();

            let mut clone_output: CloneOutput = Default::default();
            let shared_state: Rc<RefCell<_>> = Rc::new(RefCell::new(clone_output));

            let progress_state = Rc::clone(&shared_state);
            let mut t = term::stdout().ok_or("unable to create stdout term")?;
            writeln!(t, "Cloning into '{}'...", config.repo().display())?;
            let progress_fn = move |progress: Progress| -> bool {
                let res = callbacks::progress(&mut progress_state.borrow_mut(), progress);
                let curr_state = progress_state.borrow();

                if !curr_state.progress.is_empty() {
                    write!(t, "{}", &curr_state.progress).expect("");
                    let _ = t.carriage_return().expect("");
                }

                let _ = t.flush().expect("");
                res
            };

            let sideband_state = Rc::clone(&shared_state);
            let mut st = term::stdout().ok_or("unable to create stdout term")?;
            let sideband_fn = move |bytes: &[u8]| -> bool {
                let res = callbacks::sideband(&mut sideband_state.borrow_mut(), bytes);
                let curr_state = sideband_state.borrow();
                write!(st, "{}", &curr_state.sideband).expect("");
                let _ = st.carriage_return().expect("");
                let _ = st.flush().expect("");
                res
            };

            let mut remote_callbacks = callbacks::get_default();
            remote_callbacks.transfer_progress(progress_fn);
            remote_callbacks.sideband_progress(sideband_fn);
            remote_callbacks.credentials(callbacks::check_creds);

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(remote_callbacks);
            repo_builder.fetch_options(fetch_opts);

            match repo_builder.clone(origin.url(), config.repo().as_ref()) {
                Ok(repository) => Ok(repository),
                Err(e) => Err(format!("Unable to clone repository: {}", e).into()),
            }
        }
    }
}
