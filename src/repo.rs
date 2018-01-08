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
use git2::{FetchOptions, Repository};
use git2::build::RepoBuilder;
use repomon::Remote;
use std::env;
use std::path::PathBuf;

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

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(callbacks::get_default());
            repo_builder.fetch_options(fetch_opts);

            match repo_builder.clone(origin.url(), config.repo().as_ref()) {
                Ok(repository) => Ok(repository),
                Err(e) => Err(format!("Unable to clone repository: {}", e).into()),
            }
        }
    }
}
