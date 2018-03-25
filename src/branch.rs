// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! branch related operations
use bincode::{serialize, Infinite};
use callbacks::{self, CallbackOutput};
use colored::*;
use error::Result;
use futures::future::result;
use futures::sync::mpsc;
use futures::{Future, Sink};
use git2::{self, AutotagOption, Direction, FetchOptions, FetchPrune, Oid, ProxyOptions,
           Repository, Status};
use log::Logs;
use rand;
use rand::distributions::{IndependentSample, Range};
use repo::{self, Config};
use repomon::{Branch, Category, Message, Remote};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Sender type for monitor.
type SenderType = mpsc::UnboundedSender<::std::result::Result<Vec<u8>, ()>>;

/// Repository monitor configuration.
#[derive(Clone, Getters, Setters)]
pub struct MonitorConfig {
    /// The base directory to start repository discovery.
    #[get]
    basedir: String,
    /// The mpsc sender type.
    #[get]
    tx: SenderType,
    /// The slog logs.
    #[get]
    logs: Logs,
    /// The remote handle to the event loop.
    #[get]
    remote_handle: ::tokio_core::reactor::Remote,
    #[get]
    #[set = "pub"]
    /// The repository name.
    repo_name: String,
    #[get]
    #[set = "pub"]
    /// The branch we are monitoring.
    branch: Branch,
    #[get]
    #[set = "pub"]
    /// The remotes we are comparing this branch against.
    remotes: Vec<Remote>,
}

impl MonitorConfig {
    /// Create a new configuration for this monitor.
    pub fn new(
        basedir: &str,
        tx: SenderType,
        logs: Logs,
        remote_handle: ::tokio_core::reactor::Remote,
    ) -> Self {
        Self {
            basedir: basedir.to_string(),
            tx: tx,
            logs: logs,
            remote_handle: remote_handle,
            repo_name: Default::default(),
            branch: Default::default(),
            remotes: Default::default(),
        }
    }
}

/// Monitor
pub fn monitor(config: &MonitorConfig) -> Result<()> {
    try_trace!(
        config.logs().stdout(),
        "Starting monitor thread";
        "repository" => config.repo_name(),
        "branch" => format!("{}", config.branch())
    );

    // Grab so info out of the branch config
    let interval = config.branch().interval_to_ms()?;
    let branch_name = config.branch().name();
    let repo_name = config.repo_name();

    // Setup the base message.
    let mut message: Message = Default::default();
    message.set_repo(repo_name.clone());

    // Delay start up to 80% to avoid running all the same intervals
    // at the same time.
    let mut rng = rand::thread_rng();
    let between = Range::new(0, (interval * 4) / 5);
    let rand_delay: u64 = between.ind_sample(&mut rng) as u64;
    try_trace!(
        config.logs().stdout(),
        "Delaying monitor start";
        "ms" => rand_delay,
        "repository" => repo_name,
        "branch" => branch_name
    );
    thread::sleep(Duration::from_millis(rand_delay));

    // Setup some config, used to discover/clone the repository
    let mut repo_config: Config = Default::default();
    repo_config.set_basedir(PathBuf::from(config.basedir()));
    repo_config.set_repo(PathBuf::from(repo_name));
    repo_config.set_remotes(config.remotes());

    let repo = repo::discover_or_clone(&repo_config)?;
    repo::check_remotes(&repo, &repo_config)?;

    loop {
        // Add some loop specific information to the message.
        let mut msg_clone = message.clone();
        msg_clone.set_uuid(Uuid::new_v4());

        // Metrics
        let now = Instant::now();

        // Run a fetch on the remotes we are monitoring.
        for remote in config.branch().remotes() {
            let mut git_remote = repo.find_remote(remote)?;

            let mut proxy_opts = ProxyOptions::new();
            proxy_opts.auto();

            let connect_output: CallbackOutput = Default::default();
            let connect_callbacks = callbacks::get_default(connect_output)?;
            git_remote.connect_auth(Direction::Fetch, Some(connect_callbacks), Some(proxy_opts))?;

            let mut proxy_opts = ProxyOptions::new();
            proxy_opts.auto();

            let download_output: CallbackOutput = Default::default();
            let download_callbacks = callbacks::get_default(download_output)?;

            let mut fetch_opts = FetchOptions::new();
            fetch_opts.remote_callbacks(download_callbacks);
            fetch_opts.proxy_options(proxy_opts);
            fetch_opts.prune(FetchPrune::On);

            let mut valid_branchname = false;
            for remote_head in git_remote.list()? {
                let rh_name = remote_head.name();
                let mut remote_branch_name = String::from("refs/heads/");
                remote_branch_name.push_str(branch_name);

                if rh_name == remote_branch_name {
                    try_trace!(
                        config.logs.stdout(),
                        "Found matching remote branch";
                        "remote_ref" => rh_name,
                        "branch" => branch_name,
                        "repo" => repo_name
                    );
                    valid_branchname = true;
                    break;
                }
            }

            if valid_branchname {
                git_remote.download(&[branch_name], Some(&mut fetch_opts))?;

                let update_output: CallbackOutput = Default::default();
                let mut update_callbacks = callbacks::get_default(update_output)?;
                git_remote.update_tips(
                    Some(&mut update_callbacks),
                    true,
                    AutotagOption::Auto,
                    None,
                )?;
            } else {
                try_error!(
                    config.logs().stderr(),
                    "Invalid branch";
                    "repository" => repo_name,
                    "branch" => branch_name
                );
                return Err(format!("invalid branch: {}", branch_name).into());
            }
        }

        let local_branch_oid = vec![get_oid_by_spec(&repo, branch_name)?];
        let remote_oids = config
            .branch()
            .remotes()
            .iter()
            .map(|x| {
                let mut remote_name = x.clone();
                remote_name.push('/');
                remote_name.push_str(branch_name);
                remote_name
            })
            .map(|remote_name| {
                (
                    remote_name.clone(),
                    get_oid_by_spec(&repo, &remote_name).expect(""),
                )
            })
            .collect::<HashMap<String, Oid>>();

        let mut messages = BTreeMap::new();
        let mut branch: Branch = Default::default();
        branch.set_name(branch_name.to_string());

        let mut remote_messages = BTreeMap::new();

        for (local_oid, (remote_name, remote_oid)) in
            local_branch_oid.iter().cycle().zip(remote_oids.iter())
        {
            let mut remote: Remote = Default::default();
            remote.set_name(remote_name.to_string());

            let (ahead, behind) = repo.graph_ahead_behind(*local_oid, *remote_oid)?;

            if ahead > 0 || behind > 0 {
                let mut message = if ahead > 0 {
                    msg_clone.set_category(Category::Ahead);
                    format!(
                        "{}{}{}{}{}",
                        "Your branch is ahead of '".green(),
                        remote_name.green(),
                        "' by ".green(),
                        ahead.to_string().green(),
                        " commit(s)".green()
                    )
                } else {
                    String::new()
                };

                message = if behind > 0 {
                    msg_clone.set_category(Category::Behind);
                    format!(
                        "{}{}{}{}{}",
                        "Your branch is behind '".green(),
                        remote_name.green(),
                        "' by ".green(),
                        behind.to_string().green(),
                        " commit(s)".green()
                    )
                } else {
                    message
                };

                try_info!(
                    config.logs().stdout(),
                    "{}",
                    message;
                    "repository" => repo_name,
                    "branch" => branch_name
                );
                remote_messages.insert(remote, message);
            } else {
                msg_clone.set_category(Category::UpToDate);
                let message = format!("Your branch is up to date with '{}'", remote_name);
                try_trace!(
                    config.logs().stdout(),
                    "{}",
                    message;
                    "repository" => repo_name,
                    "branch" => branch_name
                );
                remote_messages.insert(remote, message);
            }
        }

        messages.insert(branch, remote_messages);
        msg_clone.set_messages(messages);

        let f = result::<(), ()>(Ok(()));
        let tx = config.tx().clone();

        config.remote_handle().spawn(|_| {
            f.then(move |_res| {
                let encoded = serialize(&msg_clone, Infinite).expect("");
                tx.send(Ok(encoded)).then(|tx| match tx {
                    Ok(_tx) => Ok(()),
                    Err(_e) => Err(()),
                })
            })
        });

        try_trace!(
            config.logs().stdout(),
            "Duration: {}.{}",
            now.elapsed().as_secs(),
            now.elapsed().subsec_millis();
            "repository" => repo_name,
            "branch" => branch_name
        );

        // Sleep until the interval has passed.
        let int: u64 = interval as u64;
        try_trace!(config.logs().stdout(), "Sleeping"; "interval" => int, "repository" => repo_name, "branch" => branch_name);
        thread::sleep(Duration::from_millis(int));
    }
}

/// Get the OID for the latest commit in the given spec.
pub fn get_oid_by_spec(repo: &Repository, spec: &str) -> Result<Oid> {
    Ok(repo.revparse_single(spec)?.id())
}

/// Convert a status to a composite string.
#[allow(dead_code)]
fn status_out(status: &Status, out: &mut String) -> Result<()> {
    let mut statuses = Vec::new();

    if status.contains(git2::Status::INDEX_NEW) {
        statuses.push("idx-new");
    }

    if status.contains(git2::Status::INDEX_MODIFIED) {
        statuses.push("idx-modified");
    }

    if status.contains(git2::Status::INDEX_DELETED) {
        statuses.push("idx-deleted");
    }

    if status.contains(git2::Status::INDEX_TYPECHANGE) {
        statuses.push("idx-typechange");
    }

    if status.contains(git2::Status::INDEX_RENAMED) {
        statuses.push("idx-renamed");
    }

    if status.contains(git2::Status::WT_NEW) {
        statuses.push("wt-new");
    }

    if status.contains(git2::Status::WT_MODIFIED) {
        statuses.push("wt-modified");
    }

    if status.contains(git2::Status::WT_DELETED) {
        statuses.push("wt-deleted");
    }

    if status.contains(git2::Status::WT_TYPECHANGE) {
        statuses.push("wt-typechange");
    }

    if status.contains(git2::Status::WT_RENAMED) {
        statuses.push("wt-renamed");
    }

    // if status.contains(git2::STATUS_WT_UNREADABLE) {
    //     statuses.push("wt-unreadable");
    // }

    if status.contains(git2::Status::IGNORED) {
        statuses.push("ignored");
    }

    if status.contains(git2::Status::CONFLICTED) {
        statuses.push("conflicted");
    }

    out.push_str(&statuses.join(", "));
    Ok(())
}
