// MIT/Apache2 License

mod builder;
use core::str::FromStr;

use alloc::vec::Vec;
pub use builder::*;
use x11rb_protocol::protocol::xproto;

/// A sans-I/O connection to an XIM server.
pub struct Xim {
    /// The current sequence number.
    sequence: u32,
    /// The connection ID for this item.
    connect_id: u16,
    /// The window used to read frames.
    client: xproto::Window,
    /// The window used to send frames.
    accept: xproto::Window,
    /// The protocol atom
    protocol: xproto::Atom,
}

/// The address to connect to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    protocol: Protocol,
    address: Vec<u8>,
}

impl Address {
    /// Create a new address.
    pub fn new(protocol: Protocol, address: Vec<u8>) -> Address {
        Address {
            protocol: protocol,
            address: address,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Protocol {
    X11,
    Tcp,
    Local,
    Decnet,
}

impl FromStr for Protocol {
    type Err = BadProtocol;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("X") {
            Ok(Protocol::X11)
        } else if s.eq_ignore_ascii_case("TCP") {
            Ok(Protocol::Tcp)
        } else if s.eq_ignore_ascii_case("LOCAL") {
            Ok(Protocol::Local)
        } else if s.eq_ignore_ascii_case("DECNET") {
            Ok(Protocol::Decnet)
        } else {
            Err(BadProtocol)
        }
    }
}

#[derive(Debug)]
pub struct BadProtocol;
