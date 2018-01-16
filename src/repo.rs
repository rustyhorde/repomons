// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` repository operations.
use callbacks::{self, CallbackOutput};
use error::Result;
use git2::{FetchOptions, Repository};
use git2::build::RepoBuilder;
use repomon::Remote;
use std::env;
use std::path::PathBuf;
use term;

/// Repository config.
#[derive(Clone, Debug, Default, Getters, Setters)]
pub struct Config<'a> {
    /// The base directory to start repository discovery/clone at.
    #[get = "pub"]
    #[set = "pub"]
    basedir: PathBuf,
    /// The repository name.
    #[get = "pub"]
    #[set = "pub"]
    repo: PathBuf,
    /// The set of remotes to fetch.
    #[get = "pub"]
    #[set = "pub"]
    remotes: &'a [Remote],
}

/// Discover the given repository at the given base directory, to try to clone it there.
pub fn discover_or_clone(config: &Config) -> Result<Repository> {
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

            let mut t = term::stdout().ok_or("unable to create stdout term")?;
            let mut clone_output: CallbackOutput = Default::default();
            let mut remote_callbacks = callbacks::get_default(clone_output)?;

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(remote_callbacks);

            repo_builder.fetch_options(fetch_opts);

            writeln!(t, "Cloning into '{}'...", config.repo().display())?;
            let repo = match repo_builder.clone(origin.url(), config.repo().as_ref()) {
                Ok(repository) => repository,
                Err(e) => return Err(format!("Unable to clone repository: {}", e).into()),
            };

            if check_remotes(&repo, config).is_ok() {
                Ok(repo)
            } else {
                Err("Unable to add remotes to repository".into())
            }
        }
    }
}

/// Check the remotes for the given repository and add if they don't exist.
pub fn check_remotes(repo: &Repository, config: &Config) -> Result<()> {
    let other_remotes: Vec<Remote> = config
        .remotes()
        .iter()
        .filter(|x| x.name() != "origin")
        .cloned()
        .collect();

    for c_remote in other_remotes {
        let name = c_remote.name();
        let url = c_remote.url();
        match repo.find_remote(name) {
            Ok(_) => {}
            Err(_e) => match repo.remote(name, url) {
                Ok(_) => {}
                Err(e) => return Err(format!("Unable to add remote '{}': {}", name, e).into()),
            },
        }
    }
    Ok(())
}
