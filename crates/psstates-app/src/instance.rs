//! Single instance handling.
//!
//! The first instance listens on a fixed loopback port. Later instances
//! connect, ask it to show its window and exit.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::AppCommand;

/// Same port the old web UI used, so the old and the new version never run
/// at the same time either.
pub const PORT: u16 = 17663;
const SHOW: &str = "psstates:show";
const ACK: &str = "psstates:ok";

pub enum Instance {
    /// We are the first instance.
    Primary(TcpListener),
    /// Another instance is running and was asked to show its window.
    Signalled,
    /// The port is taken by something else (e.g. the old Unity version).
    Blocked,
}

pub fn acquire() -> Instance {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)) {
        Ok(listener) => Instance::Primary(listener),
        Err(_) => {
            if signal_existing() {
                Instance::Signalled
            } else {
                Instance::Blocked
            }
        }
    }
}

fn signal_existing() -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(stream, "{SHOW}").is_err() {
        return false;
    }
    let mut line = String::new();
    let _ = BufReader::new(stream).read_line(&mut line);
    line.trim() == ACK
}

/// Answers "show" requests of later instances.
pub fn serve(listener: TcpListener, send: Arc<dyn Fn(AppCommand) + Send + Sync>) {
    let spawned = std::thread::Builder::new()
        .name("instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() && line.trim() == SHOW {
                    let _ = writeln!(&stream, "{ACK}");
                    tracing::info!("Another instance asked to show the window");
                    send(AppCommand::Show);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("Single instance listener unavailable: {e}");
    }
}
