//! Plain OSC over UDP.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::RwLock;

use psstates_core::ParamValue;
use rosc::{OscMessage, OscPacket, OscType};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::VRCHAT_DEFAULT_OSC_PORT;

/// A received OSC message.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingMessage {
    pub addr: String,
    pub args: Vec<OscType>,
}

impl IncomingMessage {
    /// First argument as a parameter value (float, int or bool).
    pub fn param_value(&self) -> Option<ParamValue> {
        match self.args.first()? {
            OscType::Float(f) if f.is_finite() => Some(ParamValue::Float(*f)),
            OscType::Double(d) if d.is_finite() => Some(ParamValue::Float(*d as f32)),
            OscType::Int(i) => Some(ParamValue::Int(*i)),
            OscType::Bool(b) => Some(ParamValue::Bool(*b)),
            _ => None,
        }
    }

    pub fn first_string(&self) -> Option<&str> {
        match self.args.first()? {
            OscType::String(s) => Some(s),
            _ => None,
        }
    }
}

/// Binds the receiving socket on all interfaces with a random port.
pub async fn bind_receiver() -> io::Result<(UdpSocket, u16)> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let port = socket.local_addr()?.port();
    Ok((socket, port))
}

fn flatten(packet: OscPacket, out: &mut Vec<IncomingMessage>) {
    match packet {
        OscPacket::Message(OscMessage { addr, args }) => out.push(IncomingMessage { addr, args }),
        OscPacket::Bundle(bundle) => {
            for p in bundle.content {
                flatten(p, out);
            }
        }
    }
}

/// Receives packets until the channel is closed.
pub async fn run_receiver(socket: UdpSocket, tx: mpsc::UnboundedSender<IncomingMessage>) {
    let mut buf = vec![0u8; 65_536];
    loop {
        let len = match socket.recv_from(&mut buf).await {
            Ok((len, _from)) => len,
            Err(e) => {
                // Windows reports ICMP "port unreachable" as ConnectionReset on UDP sockets.
                tracing::debug!("OSC receive error: {e}");
                continue;
            }
        };
        match rosc::decoder::decode_udp(&buf[..len]) {
            Ok((_, packet)) => {
                let mut msgs = Vec::new();
                flatten(packet, &mut msgs);
                for m in msgs {
                    if tx.send(m).is_err() {
                        return;
                    }
                }
            }
            Err(e) => tracing::debug!("Invalid OSC packet: {e:?}"),
        }
    }
}

/// Sends OSC messages to VRChat.
#[derive(Debug)]
pub struct OscSender {
    socket: StdUdpSocket,
    target: RwLock<SocketAddr>,
}

impl OscSender {
    pub fn new() -> io::Result<Self> {
        let socket = StdUdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        Ok(Self {
            socket,
            target: RwLock::new(SocketAddr::from((
                Ipv4Addr::LOCALHOST,
                VRCHAT_DEFAULT_OSC_PORT,
            ))),
        })
    }

    pub fn target(&self) -> SocketAddr {
        *self.target.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_target(&self, addr: SocketAddr) {
        *self.target.write().unwrap_or_else(|e| e.into_inner()) = addr;
    }

    pub fn send(&self, addr: &str, value: ParamValue) -> io::Result<()> {
        let arg = match value {
            ParamValue::Float(f) => OscType::Float(f),
            ParamValue::Int(i) => OscType::Int(i),
            ParamValue::Bool(b) => OscType::Bool(b),
        };
        let packet = OscPacket::Message(OscMessage {
            addr: addr.to_string(),
            args: vec![arg],
        });
        let bytes = rosc::encoder::encode(&packet)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))?;
        self.socket.send_to(&bytes, self.target())?;
        Ok(())
    }

    /// Sends all messages, returning the number successfully sent.
    pub fn send_all<'a>(&self, msgs: impl IntoIterator<Item = &'a (String, ParamValue)>) -> usize {
        msgs.into_iter()
            .filter(|(addr, value)| match self.send(addr, *value) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("Failed to send OSC message {addr}: {e}");
                    false
                }
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn send_and_receive_roundtrip() {
        let (socket, port) = bind_receiver().await.unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::spawn(run_receiver(socket, tx));

        let sender = OscSender::new().unwrap();
        sender.set_target(SocketAddr::from((Ipv4Addr::LOCALHOST, port)));
        sender
            .send("/avatar/parameters/X", ParamValue::Bool(true))
            .unwrap();
        sender
            .send("/avatar/parameters/Y", ParamValue::Float(0.5))
            .unwrap();

        let a = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.addr, "/avatar/parameters/X");
        assert_eq!(a.param_value(), Some(ParamValue::Bool(true)));
        let b = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(b.param_value(), Some(ParamValue::Float(0.5)));
    }
}
