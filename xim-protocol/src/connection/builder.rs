// MIT/Apache2 License

use alloc::{string::String, borrow::Cow};
use x11rb_protocol::protocol::xproto;

const XIM_SERVERS: &str = "XIM_SERVERS";
const XIM_LOCALES: &str = "LOCALES";
const XIM_TRANSPORT: &str = "TRANSPORT";
const XIM_PROTOCOL: &str = "_XIM_PROTOCOL";
const XIM_XCONNECT: &str = "_XIM_XCONNECT";

const NECESSARY_ATOMS: [&str; 5] = [
    XIM_SERVERS,
    XIM_LOCALES,
    XIM_TRANSPORT,
    XIM_PROTOCOL,
    XIM_XCONNECT,
];
const NUM_NECESSARY_ATOMS: usize = NECESSARY_ATOMS.len();

struct Core {
    screen: usize,
    root: xproto::Window,
    name: String,
    atoms: [xproto::Atom; NUM_NECESSARY_ATOMS],
}

/// A struct to be used to build an XIM connection.
pub struct XimBuilder {
    // the screen to use
    screen: usize,
    // the root to use
    root: xproto::Window,
    // the name of the IM server
    name: String,
    // the atoms we need to load
    atoms: [AtomSlot; NUM_NECESSARY_ATOMS],
    // change out the event mask
    changed_evmask: bool,
}

#[derive(Copy, Clone)]
enum AtomSlot {
    /// Not yet retrieved.
    Unset(&'static str),
    /// Retrieved
    Retrieved(xproto::Atom),
}

impl XimBuilder {
    pub fn new(screen: usize, setup: &xproto::Setup, name: String) -> XimBuilder {
        XimBuilder {
            screen,
            root: setup.roots[screen].root,
            name,
            atoms: [
                AtomSlot::Unset(XIM_SERVERS),
                AtomSlot::Unset(XIM_LOCALES),
                AtomSlot::Unset(XIM_TRANSPORT),
                AtomSlot::Unset(XIM_PROTOCOL),
                AtomSlot::Unset(XIM_XCONNECT),
            ],
            changed_evmask: false,
        }
    }

    /// Iterate over the atoms requests that we need to send.
    pub fn intern_atom_requests(&self) -> impl Iterator<Item = (usize, xproto::InternAtomRequest<'static>)> + '_ {
        self.atoms.iter().filter_map(|slot| {
            match slot {
                AtomSlot::Unset(name) => Some(xproto::InternAtomRequest {
                    name: Cow::Borrowed(name.as_bytes()),
                    only_if_exists: false,
                }),
                _ => None,
            }
        }).enumerate()
    }

    /// Set an atom that has been interned.
    pub fn use_atom(&mut self, index: usize, atom: xproto::Atom) {
        self.atoms[index] = AtomSlot::Retrieved(atom);
    }

    /// Get an attribute modifying request that needs to be applied to the root window.
    pub fn change_event_mask(&mut self) -> xproto::EventMask {
        self.changed_evmask = true;
        xproto::EventMask::PROPERTY_CHANGE
    }

    /// Once we're done, move onto the next stage.
    pub fn next_stage(self) -> StageGetServers {
        let Self { screen, root, name, atoms, changed_evmask } = self;
        todo!()
    }
}

/// The stage where we get servers.
pub struct StageGetServers {
    core: Core,
}