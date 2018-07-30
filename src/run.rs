// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomon` runtime
use branch::{self, MonitorConfig};
use clap::{App, Arg};
use error::Result;
use futures::sync::mpsc;
use futures::{Future, Stream};
use log::Logs;
use repomon;
use slog::Level;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::rc::Rc;
use std::thread;
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;
use tokio_io::io::write_all;
use tokio_io::AsyncRead;

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
        ).arg(
            Arg::with_name("address")
                .short("a")
                .long("address")
                .takes_value(true)
                .required(true)
                .default_value("127.0.0.1:8080"),
        ).arg(
            Arg::with_name("verbose")
                .short("v")
                .multiple(true)
                .help("Set the output verbosity level (more v's = more verbose)"),
        ).arg(
            Arg::with_name("quiet")
                .short("q")
                .long("quiet")
                .multiple(true)
                .conflicts_with("verbose")
                .help("Restrict output.  (more q's = more quiet"),
        ).arg(Arg::with_name("repo").default_value("."))
        .get_matches();

    // Setup the logging (info by default)
    let mut level = match matches.occurrences_of("verbose") {
        0 => Level::Info,
        1 => Level::Debug,
        2 | _ => Level::Trace,
    };

    level = match matches.occurrences_of("quiet") {
        0 => level,
        1 => Level::Warning,
        2 => Level::Error,
        3 | _ => Level::Critical,
    };

    let mut logs: Logs = Default::default();
    logs.set_stdout_level(level);

    // Logging clones for server, monitor threads, receiver, and config.
    let server_logs = logs.clone();
    let thread_logs = logs.clone();
    let receiver_logs = logs.clone();
    let config_logs = logs.clone();
    let core_logs = logs.clone();

    try_trace!(logs.stdout(), "Logging configured!");

    let config_file = File::open(matches.value_of("config").ok_or("invalid config file")?)?;
    let mut reader = BufReader::new(config_file);
    let repomon = repomon::read_toml(&mut reader)?;

    try_trace!(logs.stdout(), "Configuration TOML parsed!");

    let addr = matches
        .value_of("address")
        .ok_or("invalid address")?
        .parse::<SocketAddr>()?;
    let mut core = Core::new()?;
    let remote_handle = core.remote();
    let handle = core.handle();
    let socket = TcpListener::bind(&addr, &handle)?;
    try_trace!(logs.stdout(), "Listening for connections"; "addr" => format!("{}", addr));

    // This is a single-threaded server, so we can just use Rc and RefCell to
    // store the map of all connections we know about.
    let connections = Rc::new(RefCell::new(HashMap::new()));

    // Clone some conns for the worker and for the server to reference.
    let rx_cons = Rc::clone(&connections);
    let srv_cons = Rc::clone(&connections);

    let srv = socket.incoming().for_each(move |(stream, addr)| {
        try_trace!(server_logs.stdout(), "Connection opened"; "addr" => format!("{}", addr));
        // We currently don't accept input from the clients, so only grabbing the writer.
        let (_r, writer) = stream.split();

        // Create a channel for our stream, which other sockets will use to
        // send us messages. Then register our address with the stream to send
        // data to us.
        let (tx, rx) = mpsc::unbounded();
        connections.borrow_mut().insert(addr, tx);

        // Whenever we receive a string on the Receiver, we write it to
        // `WriteHalf<TcpStream>`.
        let writer_logs = logs.clone();
        let socket_writer = rx.fold(writer, move |writer, msg: Vec<u8>| {
            try_trace!(writer_logs.stdout(), "Sending bincoded monitor"; "addr" => format!("{}", addr));
            let writer_buf_tuple = write_all(writer, msg);
            let writer = writer_buf_tuple.map(|(writer, _)| writer);
            writer.map_err(|_| ())
        });

        // Make the socket write future into a future that can be spawned.
        let socket_writer = socket_writer.map(|_| ());

        let connections = Rc::clone(&srv_cons);
        let spawn_logs = server_logs.clone();
        handle.spawn(socket_writer.then(move |_| {
            try_trace!(spawn_logs.stdout(), "Closing connection"; "addr" => format!("{}", addr));
            connections.borrow_mut().remove(&addr);
            Ok(())
        }));

        Ok(())
    });

    let srv = srv.map_err(|_| ());

    // The tx gets cloned into monitor threads for sending messages.
    // The rx send received messages to connected clients.
    let (tx, rx) = mpsc::unbounded();
    let basedir = repomon.basedir();

    let mut monitor_config = MonitorConfig::new(basedir, tx, config_logs, remote_handle);

    // Startup the monitor threads (one per repository/branch combination).
    for (repo_name, repo) in repomon.repos() {
        for branch in repo.branch() {
            // All the clones.  Moving into monitor thread.
            let t_logs = thread_logs.clone();
            let t_repo_name = repo_name.clone();
            let t_branch_name = branch.name().clone();
            monitor_config.set_repo_name(repo_name.clone());
            monitor_config.set_branch(branch.clone());
            monitor_config.set_remotes(repo.remotes().clone());

            let t_monitor_config = monitor_config.clone();

            thread::spawn(move || {
                if let Err(e) = branch::monitor(&t_monitor_config) {
                    try_error!(
                        t_logs.stderr(),
                        "Error starting monitor: {}", e;
                        "repository" => t_repo_name,
                        "branch" => t_branch_name
                    );
                }
            });
        }
    }

    // This is where we send messages from the monitors off to any connected clients.
    let rx_fut = rx.for_each(|message_result| {
        match message_result {
            Ok(message) => {
                let mut conns = rx_cons.borrow_mut();
                for tx in conns.iter_mut().map(|(_, v)| v) {
                    if tx.unbounded_send(message.clone()).is_err() {
                        try_error!(receiver_logs.stderr(), "Error sending message");
                    }
                }
            }
            Err(()) => try_error!(receiver_logs.stderr(), "Error"),
        }
        Ok(())
    });

    // Join the server and monitor futures.
    let both = rx_fut.join(srv);

    try_info!(core_logs.stdout(), "Starting repomons...");
    // Run the monitors and the server.
    core.run(both).expect("Failed to run event loop");

    Ok(0)
}
