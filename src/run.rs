// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` runtime
use bincode::{serialize, Infinite};
use branch;
use clap::{App, Arg};
use error::Result;
use futures::future::result;
use futures::sync::mpsc;
use futures::{Future, Sink, Stream};
use git2::{self, BranchType, CredentialType, FetchOptions, FetchPrune, Progress, RemoteCallbacks,
           Repository, Status, StatusOptions};
use git2::Cred;
use rand;
use rand::distributions::{IndependentSample, Range};
use repomon::{self, Branch, Message};
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::time::Duration;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::net::SocketAddr;
use std::rc::Rc;
use std::thread;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Remote};
use tokio_io::AsyncRead;
use tokio_io::io::write_all;

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

/// Convert a status to a composite string.
fn status_out(status: &Status, out: &mut String) -> Result<()> {
    let mut statuses = Vec::new();

    if status.contains(git2::STATUS_INDEX_NEW) {
        statuses.push("idx-new");
    }

    if status.contains(git2::STATUS_INDEX_MODIFIED) {
        statuses.push("idx-modified");
    }

    if status.contains(git2::STATUS_INDEX_DELETED) {
        statuses.push("idx-deleted");
    }

    if status.contains(git2::STATUS_INDEX_TYPECHANGE) {
        statuses.push("idx-typechange");
    }

    if status.contains(git2::STATUS_INDEX_RENAMED) {
        statuses.push("idx-renamed");
    }

    if status.contains(git2::STATUS_WT_NEW) {
        statuses.push("wt-new");
    }

    if status.contains(git2::STATUS_WT_MODIFIED) {
        statuses.push("wt-modified");
    }

    if status.contains(git2::STATUS_WT_DELETED) {
        statuses.push("wt-deleted");
    }

    if status.contains(git2::STATUS_WT_TYPECHANGE) {
        statuses.push("wt-typechange");
    }

    if status.contains(git2::STATUS_WT_RENAMED) {
        statuses.push("wt-renamed");
    }

    // if status.contains(git2::STATUS_WT_UNREADABLE) {
    //     statuses.push("wt-unreadable");
    // }

    if status.contains(git2::STATUS_IGNORED) {
        statuses.push("ignored");
    }

    if status.contains(git2::STATUS_CONFLICTED) {
        statuses.push("conflicted");
    }

    out.push_str(&statuses.join(", "));
    Ok(())
}

/// CLI Runtime
pub fn run() -> Result<i32> {
    let matches = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Monitors a set of repositories for changes to branches")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .required(true)
                .default_value(".repomon.toml"),
        )
        .arg(Arg::with_name("repo").default_value("."))
        .get_matches();

    let config_file = File::open(matches.value_of("config").ok_or("invalid config file")?)?;
    let mut reader = BufReader::new(config_file);
    let branches = repomon::read_toml(&mut reader)?;

    let mut core = Core::new()?;
    let remote_handle = core.remote();
    let handle = core.handle();
    let socket = TcpListener::bind(&"0.0.0.0:8080".parse::<SocketAddr>()?, &handle)?;

    // This is a single-threaded server, so we can just use Rc and RefCell to
    // store the map of all connections we know about.
    let connections = Rc::new(RefCell::new(HashMap::new()));

    // Clone some conns for the worker and for the server to reference.
    let rx_cons = Rc::clone(&connections);
    let srv_cons = Rc::clone(&connections);

    let srv = socket.incoming().for_each(move |(stream, addr)| {
        let (_r, writer) = stream.split();

        // Create a channel for our stream, which other sockets will use to
        // send us messages. Then register our address with the stream to send
        // data to us.
        let (tx, rx) = mpsc::unbounded();
        connections.borrow_mut().insert(addr, tx);

        // Whenever we receive a string on the Receiver, we write it to
        // `WriteHalf<TcpStream>`.
        let socket_writer = rx.fold(writer, move |writer, msg: Vec<u8>| {
            writeln!(io::stdout(), "Sending bincoded monitor to {}", &addr).expect("");
            let writer_buf_tuple = write_all(writer, msg);
            let writer = writer_buf_tuple.map(|(writer, _)| writer);
            writer.map_err(|_| ())
        });

        // Make the socket write future into a future that can be spawned.
        let socket_writer = socket_writer.map(|_| ());

        let connections = Rc::clone(&srv_cons);
        handle.spawn(socket_writer.then(move |_| {
            writeln!(io::stdout(), "Closing connection: {}", &addr).expect("");
            connections.borrow_mut().remove(&addr);
            Ok(())
        }));

        Ok(())
    });

    let (tx, rx) = mpsc::channel(1);

    for (repo, branches) in branches.branch_map() {
        let t_tx = tx.clone();
        let t_repo = repo.clone();
        let t_branches = branches.clone();
        let t_remote = remote_handle.clone();
        thread::spawn(move || monitor_repo(&t_repo, &t_branches, &t_tx, &t_remote));
    }

    let rx_fut = rx.for_each(|res| {
        match res {
            Ok(repo) => {
                writeln!(io::stdout(), "Success: Sending bincoded Monitor").expect("");
                let mut conns = rx_cons.borrow_mut();
                for tx in conns.iter_mut().map(|(_, v)| v) {
                    if tx.unbounded_send(repo.clone()).is_err() {
                        writeln!(io::stdout(), "Error sending message").expect("");
                    }
                }
            }
            Err(_) => writeln!(io::stdout(), "Error").expect(""),
        }
        Ok(())
    });

    let srv = srv.map_err(|_| ());
    let both = rx_fut.join(srv);

    core.run(both).expect("failed to run event loop");

    let repo = Repository::discover(matches.value_of("repo").ok_or("")?)?;
    let mut status_opts = StatusOptions::new();
    status_opts.include_ignored(false);
    status_opts.include_untracked(true);

    let statuses = repo.statuses(Some(&mut status_opts))?;

    let mut rcb = RemoteCallbacks::new();
    rcb.transfer_progress(progress);
    rcb.sideband_progress(side_band);
    rcb.credentials(check_creds);

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(rcb);
    fetch_opts.prune(FetchPrune::On);

    let master_oid = branch::get_oid_by_branch_name(&repo, "master", Some(BranchType::Local))?;
    let origin_master_oid =
        branch::get_oid_by_branch_name(&repo, "origin/master", Some(BranchType::Remote))?;
    let (ahead, behind) = repo.graph_ahead_behind(master_oid, origin_master_oid)?;

    if ahead > 0 {
        writeln!(
            io::stdout(),
            "Your branch is ahead of '{}' by {} commit(s)",
            "origin/master",
            ahead
        )?;
    } else if behind > 0 {
        writeln!(
            io::stdout(),
            "Your branch is behind '{}' by {} commit(s)",
            "origin/master",
            behind
        )?;
    } else {
        writeln!(
            io::stdout(),
            "Your branch is up to date with '{}'",
            "origin/master"
        )?;
    }

    for (branch, _) in repo.branches(None)?
        .filter_map(|branch_res| branch_res.ok())
    {
        writeln!(io::stdout(), "Branch: {}", branch.name()?.ok_or("No name")?)?;
        writeln!(io::stdout(), "Branch is head: {}", branch.is_head())?;
        writeln!(
            io::stdout(),
            "Branch OID: {}",
            branch.get().target().ok_or("No OID")?
        )?;
    }
    for status in statuses.iter() {
        let mut status_str = String::new();
        status_out(&status.status(), &mut status_str)?;
        writeln!(
            io::stdout(),
            "Path: {}, {}",
            status.path().unwrap_or("''"),
            status_str
        )?;
    }

    for remote in repo.remotes()?.iter().filter_map(|remote_opt| remote_opt) {
        repo.find_remote(remote)?
            .fetch(&["master"], Some(&mut fetch_opts), None)?;
    }
    Ok(0)
}

/// Monitor the repository branches.
fn monitor_repo(
    repo: &str,
    branches: &[Branch],
    tx: &mpsc::Sender<::std::result::Result<Vec<u8>, ()>>,
    remote: &Remote,
) {
    let mut rng = rand::thread_rng();
    let between = Range::new(1000, 5000);
    let mut message: Message = Default::default();
    message.set_repo(repo.to_string());
    message.set_branch(branches[0].clone());
    message.set_count(0);

    loop {
        let tx = tx.clone();
        let mut msg_clone = message.clone();
        let n: u64 = between.ind_sample(&mut rng);
        msg_clone.set_count(TryFrom::try_from(n).expect("cannot convert to u32"));
        thread::sleep(Duration::from_millis(n));
        let f = result::<(), ()>(Ok(()));

        remote.spawn(|_| {
            f.then(move |_res| {
                let encoded = serialize(&msg_clone, Infinite).expect("");
                tx.send(Ok(encoded)).then(|tx| match tx {
                    Ok(_tx) => Ok(()),
                    Err(_e) => Err(()),
                })
            })
        });
    }
}
