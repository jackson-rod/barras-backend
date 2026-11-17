//! A port of this machine where nothing listens.

use std::net::{Ipv4Addr, TcpListener};

/// A port of `127.0.0.1` where nothing listens: the system picks one that is free for a listener
/// of this function's own, which is closed before it returns. A connection to it is refused at
/// once, unless another process takes the port first.
pub fn closed_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .unwrap_or_else(|error| panic!("cannot find a free port of 127.0.0.1: {error}"))
}
