// MIT/Apache2 License

// TODO: this is sloppy, redo as one big struct

use crate::protocol::{ConnectFr, ConnectReplyFr, OpenFr, OpenReplyFr, XimFrame};

use super::{Address, BadProtocol, Protocol};
use alloc::{
    borrow::Cow,
    string::String,
    vec::{IntoIter as VecIter, Vec},
};
use core::str;
use x11rb_protocol::protocol::{xproto::{self, GetPropertyReply, GetPropertyRequest}, Event};

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

const SERVERS_INDEX: usize = 0;
const TRANSPORT_INDEX: usize = 2;
const PROTOCOL_INDEX: usize = 3;
const CONNECT_INDEX: usize = 4;

struct Core {
    screen: usize,
    root: xproto::Screen,
    name: String,
    atoms: [xproto::Atom; NUM_NECESSARY_ATOMS],
}

/// A struct to be used to build an XIM connection.
pub struct XimBuilder {
    // the screen to use
    screen: usize,
    // the root to use
    root: xproto::Screen,
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
            root: setup.roots[screen].clone(),
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
    pub fn intern_atom_requests(
        &self,
    ) -> impl Iterator<Item = (usize, xproto::InternAtomRequest<'static>)> + '_ {
        self.atoms
            .iter()
            .filter_map(|slot| match slot {
                AtomSlot::Unset(name) => Some(xproto::InternAtomRequest {
                    name: Cow::Borrowed(name.as_bytes()),
                    only_if_exists: false,
                }),
                _ => None,
            })
            .enumerate()
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
    ///
    /// This also provides the necessary information to build the servers.
    pub fn next_stage(self) -> (StageGetServers, xproto::GetPropertyRequest) {
        let Self {
            screen,
            root,
            name,
            atoms,
            changed_evmask,
        } = self;
        let atoms = atoms.map(|slot| match slot {
            AtomSlot::Retrieved(atom) => atom,
            _ => panic!("Missing atoms!"),
        });
        if !changed_evmask {
            panic!("Missing event mask!");
        }

        let core = Core {
            screen,
            root,
            name,
            atoms,
        };
        let req = GetPropertyRequest {
            delete: false,
            window: core.root.root,
            property: atoms[SERVERS_INDEX],
            type_: xproto::AtomEnum::ATOM.into(),
            long_length: 10000,
            long_offset: 0,
        };

        (StageGetServers { core }, req)
    }
}

/// The stage where we get servers.
pub struct StageGetServers {
    core: Core,
}

impl StageGetServers {
    /// Indicate that we have retrieved the servers.
    pub fn retrieved_servers(
        self,
        reply: xproto::GetPropertyReply,
    ) -> Result<StageCheckServers, ConnectError> {
        if reply.format != 32 || reply.type_ != xproto::AtomEnum::ATOM.into() {
            return Err(ConnectError::RetrievedServersNotAtoms);
        }

        // collect all of the atoms into an array
        let atoms = bytemuck::allocation::pod_collect_to_vec::<_, u32>(
            &reply.value[..reply.length as usize * 4],
        );

        Ok(StageCheckServers {
            core: self.core,
            servers: atoms.into_iter(),
            check_state: CheckState::Initial,
            current_server: None,
            last_err: None,
        })
    }
}

/// The stage where we check servers.
pub struct StageCheckServers {
    core: Core,
    servers: VecIter<u32>,
    check_state: CheckState,
    current_server: Option<CurrentServer>,
    last_err: Option<ConnectError>,
}

struct CurrentServer {
    server: xproto::Atom,
    owner_window: Option<xproto::Window>,
    requestor_window: Option<xproto::Window>,
    client_window: Option<xproto::Window>,
    accept_window: Option<xproto::Window>,
    address: Option<Address>,
    connect_id: Option<u16>,
}

impl CurrentServer {
    fn owner_window(&self) -> xproto::Window {
        self.owner_window.expect("No owner window!")
    }

    fn requestor_window(&self) -> xproto::Window {
        self.requestor_window.expect("No requestor window!")
    }

    fn client_window(&self) -> xproto::Window {
        self.client_window.expect("No client window!")
    }

    fn accept_window(&self) -> xproto::Window {
        self.accept_window.expect("No accept window!")
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum CheckState {
    /// The starting state, where we're not processing a server.
    Initial,
    /// Just sent out querying for this server atom.
    Querying,
    /// Generating an XID for the requestor window.
    GeneratingXidForRequestor,
    /// Running the CreateWindow request.
    CreatingWindow,
    /// Deleting the requestor window.
    DeletingWindow,
    /// Convert the window selection.
    ConvertSelection,
    /// Wait for a SelectionNotify event.
    WaitForSelNotify,
    /// The user gets the "transport" property.
    GetTransport,
    /// Delete the window, but this time just because we don't
    /// really need it anymore.
    WindowNotNeeded,
    /// Generate an XID for the client window.
    GeneratingXidForClient,
    /// Running the CreateWindow request.
    CreatingClientWindow,
    /// Sending an event the the root window involving the client window.
    InvolvingClientWindow,
    /// Waiting for the server to confirm.
    WaitForClientNotify,
    /// Connect to the server.
    EstablishingConnection,
    /// Send a connect frame.
    SendConnectFrame,
    /// Wait for the connect_reply frame.
    WaitForConnectReply,
    /// Send the open frame.
    SendOpenFrame,
    /// Wait for the open_reply frame.
    WaitForOpenReply,
    /// Connection is complete.
    Complete,
}

/// A directive that the user needs to fulfill while using the builder.
pub enum ServerCheckDirective {
    /// The user needs to provide a server.
    FetchAtomInfo(xproto::GetSelectionOwnerRequest, xproto::GetAtomNameRequest),
    /// The user needs to generate an XID.
    GenerateXid,
    /// The user needs to create a window.
    CreateWindow(xproto::CreateWindowRequest<'static>),
    /// The user needs to delete a window.
    DeleteWindow(xproto::DestroyWindowRequest),
    /// The user needs to convert a selection.
    ConvertSelection(xproto::ConvertSelectionRequest),
    /// Feed us a `SelectionNotifyEvent`.
    GetSelectionNotify,
    /// Feed us a `ClientMessageEvent`.
    GetClientMessage,
    /// The user needs to get a property.
    GetProperty(GetPropertyRequest),
    /// The user needs to send an event.
    SendEvent(xproto::SendEventRequest<'static>),
    /// The user needs to send a frame.
    SendFrame {
        frame: XimFrame<'static>,
        window: xproto::Window,
        protocol_atom: xproto::Atom,
        sequence: u32,
    },
    /// The user needs to receive a frame.
    RecvFrame,
    /// The user needs to connect to the XIM server over the
    /// given address.
    Connect(Address),
}

/// The result of completing a `ServerCheckDirective`.
pub enum Completion<'a> {
    AtomCheck(xproto::GetSelectionOwnerReply, xproto::GetAtomNameReply),
    XidGenerated(xproto::Window),
    RequestCompleted,
    SelectionNotifyEvent(xproto::SelectionNotifyEvent),
    ClientMessageEvent(xproto::ClientMessageEvent),
    Property(xproto::GetPropertyReply),
    ConnectReply(ConnectReplyFr),
    OpenReply(OpenReplyFr<'a>),
}

impl StageCheckServers {
    fn current_server(&self) -> &CurrentServer {
        self.current_server.as_ref().expect("No current server!")
    }

    fn current_server_mut(&mut self) -> &mut CurrentServer {
        self.current_server.as_mut().expect("No current server!")
    }

    /// Get the next directive.
    pub fn next_directive(&mut self) -> Option<ServerCheckDirective> {
        loop {
            match self.check_state {
                CheckState::Initial => {
                    // note that vec::IntoIter is fused so we don't have to worry about looping it
                    let next_server = self.servers.next()?.into();
                    self.current_server = Some(CurrentServer {
                        server: next_server,
                        owner_window: None,
                        requestor_window: None,
                        client_window: None,
                        accept_window: None,
                        address: None,
                        connect_id: None,
                    });
                    self.check_state = CheckState::Querying;
                }
                CheckState::Querying => {
                    // return the requests we need
                    let atom = self.current_server().server;
                    return Some(ServerCheckDirective::FetchAtomInfo(
                        xproto::GetSelectionOwnerRequest { selection: atom },
                        xproto::GetAtomNameRequest { atom },
                    ));
                }
                CheckState::GeneratingXidForRequestor | CheckState::GeneratingXidForClient => {
                    // return the requests we need
                    return Some(ServerCheckDirective::GenerateXid);
                }
                CheckState::CreatingWindow => {
                    return Some(ServerCheckDirective::CreateWindow(
                        xproto::CreateWindowRequest {
                            depth: 0,
                            wid: self.current_server().requestor_window(),
                            parent: self.core.root.root,
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                            border_width: 1,
                            class: xproto::WindowClass::INPUT_OUTPUT,
                            visual: self.core.root.root_visual,
                            value_list: Cow::Owned(xproto::CreateWindowAux::new()),
                        },
                    ));
                }
                CheckState::DeletingWindow | CheckState::WindowNotNeeded => {
                    return Some(ServerCheckDirective::DeleteWindow(
                        xproto::DestroyWindowRequest {
                            window: self.current_server().requestor_window(),
                        },
                    ));
                }
                CheckState::ConvertSelection => {
                    let current_server = self.current_server();
                    return Some(ServerCheckDirective::ConvertSelection(
                        xproto::ConvertSelectionRequest {
                            requestor: current_server.requestor_window(),
                            selection: current_server.server,
                            target: self.core.atoms[TRANSPORT_INDEX],
                            property: self.core.atoms[TRANSPORT_INDEX],
                            time: 0, // current time
                        },
                    ));
                }
                CheckState::WaitForSelNotify => {
                    return Some(ServerCheckDirective::GetSelectionNotify)
                }
                CheckState::GetTransport => {
                    return Some(ServerCheckDirective::GetProperty(
                        xproto::GetPropertyRequest {
                            delete: true,
                            window: self.current_server().requestor_window(),
                            property: self.core.atoms[TRANSPORT_INDEX],
                            type_: self.core.atoms[TRANSPORT_INDEX],
                            long_length: 10000,
                            long_offset: 0,
                        },
                    ));
                }
                CheckState::CreatingClientWindow => {
                    return Some(ServerCheckDirective::CreateWindow(
                        xproto::CreateWindowRequest {
                            depth: 0,
                            wid: self.current_server().client_window(),
                            parent: self.core.root.root,
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                            border_width: 1,
                            class: xproto::WindowClass::INPUT_OUTPUT,
                            visual: self.core.root.root_visual,
                            value_list: Cow::Owned(xproto::CreateWindowAux::new()),
                        },
                    ))
                }
                CheckState::InvolvingClientWindow => {
                    // we need to send a message
                    let data = [self.current_server().client_window(), 0, 0, 0, 0];
                    let message = xproto::ClientMessageEvent {
                        response_type: xproto::CLIENT_MESSAGE_EVENT,
                        format: 32,
                        sequence: 0,
                        type_: self.core.atoms[CONNECT_INDEX],
                        window: self.current_server().owner_window(),
                        data: data.into(),
                    };

                    return Some(ServerCheckDirective::SendEvent(xproto::SendEventRequest {
                        propagate: false,
                        destination: self.current_server().owner_window(),
                        event_mask: xproto::EventMask::NO_EVENT.into(),
                        event: Cow::Owned(message.into()),
                    }));
                }
                CheckState::WaitForClientNotify => {
                    return Some(ServerCheckDirective::GetClientMessage);
                }
                CheckState::EstablishingConnection => {
                    return Some(ServerCheckDirective::Connect(
                        self.current_server()
                            .address
                            .as_ref()
                            .cloned()
                            .expect("no address"),
                    ));
                }
                CheckState::SendConnectFrame => {
                    return Some(ServerCheckDirective::SendFrame {
                        frame: ConnectFr {
                            byte_order: BYTE_ORDER,
                            client_major_protocol_version: 0,
                            client_minor_protocol_version: 0,
                            client_auth_protocol_names: alloc::vec![],
                        }
                        .into(),
                        window: self.current_server().accept_window(),
                        protocol_atom: self.core.atoms[PROTOCOL_INDEX],
                        sequence: 0,
                    });
                }
                CheckState::WaitForConnectReply | CheckState::WaitForOpenReply => {
                    return Some(ServerCheckDirective::RecvFrame)
                }
                CheckState::SendOpenFrame => {
                    return Some(ServerCheckDirective::SendFrame {
                        frame: OpenFr {
                            field1: Default::default(),
                        }
                        .into(),
                        window: self.current_server().accept_window(),
                        protocol_atom: self.core.atoms[PROTOCOL_INDEX],
                        sequence: 1,
                    })
                }
                CheckState::Complete => return None,
            }
        }
    }

    /// Indicate that an error has occurred and that we should ignore this server.
    pub fn early_out(&mut self) {
        tracing::debug!("Received early out signal, moving on to next error.");

        if self.check_state > CheckState::CreatingWindow
            && self.check_state < CheckState::WindowNotNeeded
        {
            // we need to safely delete the requestor window
            self.check_state = CheckState::DeletingWindow;
        } else {
            // empty out server and reset
            self.current_server = None;
            self.check_state = CheckState::Initial;
        }
    }

    /// Indicate that we've completed the atom check.
    pub fn complete_atom_check(
        &mut self,
        sel_reply: xproto::GetSelectionOwnerReply,
        atom_reply: xproto::GetAtomNameReply,
    ) {
        match self.check_state {
            CheckState::Querying => {
                // short-circuit out if the atom's name doesn't match our target name
                tracing::debug!("found atom name: {:?}", &atom_reply.name);
                if atom_reply.name != self.core.name.as_bytes() {
                    self.last_err = Some(ConnectError::AtomNameMismatch);
                    self.early_out();
                    return;
                }

                // begin creating the requestor window
                self.current_server_mut().owner_window = Some(sel_reply.owner);
                self.check_state = CheckState::GeneratingXidForRequestor;
            }
            _ => panic!("wasn't expecting atom check"),
        }
    }

    /// Indicate that we've generated an XID.
    pub fn complete_xid(&mut self, id: u32) {
        match self.check_state {
            CheckState::GeneratingXidForRequestor => {
                // begin creating the requestor window
                self.current_server_mut().requestor_window = Some(id);
                self.check_state = CheckState::CreatingWindow;
            }
            _ => panic!("wasn't expecting xid generation"),
        }
    }

    /// Indicate that we've completed a request or frame that doesn't have an output.
    pub fn complete_request(&mut self) {
        match self.check_state {
            CheckState::CreatingWindow => {
                // move on to the next stage now that requestor_window is complete
                self.check_state = CheckState::ConvertSelection;
            }
            CheckState::DeletingWindow => {
                // start over again
                self.current_server = None;
                self.check_state = CheckState::Initial;
            }
            CheckState::ConvertSelection => {
                // now we wait for a selection notify event
                self.check_state = CheckState::WaitForSelNotify;
            }
            CheckState::WindowNotNeeded => {
                // request window is now deleted
                self.current_server_mut().requestor_window = None;

                // move onto opening the connection
                self.check_state = CheckState::GeneratingXidForClient;
            }
            CheckState::CreatingClientWindow => {
                // move into the next stage
                self.check_state = CheckState::WaitForClientNotify;
            }
            CheckState::SendConnectFrame => {
                self.check_state = CheckState::WaitForConnectReply;
            }
            CheckState::SendOpenFrame => {
                self.check_state = CheckState::Complete;
            }
            _ => panic!("wasn't expecting request completion"),
        }
    }

    /// Indicate that we found the selection notify event.
    pub fn complete_selection_notify(
        &mut self,
        notify: xproto::SelectionNotifyEvent,
    ) -> Result<(), xproto::SelectionNotifyEvent> {
        match self.check_state {
            CheckState::WaitForSelNotify => {
                // check to see if we want this selection
                if notify.requestor != self.current_server().requestor_window() {
                    return Err(notify);
                }

                if notify.selection == xproto::AtomEnum::NONE.into() {
                    // this is an error.
                    self.last_err = Some(ConnectError::NoServerAtom);
                    self.early_out();
                    return Ok(());
                }

                // we want this selection
                self.check_state = CheckState::GetTransport;
                Ok(())
            }
            _ => Err(notify),
        }
    }

    /// Indicate that we've gotten a new client notification.
    pub fn complete_client_message(
        &mut self,
        message: xproto::ClientMessageEvent,
    ) -> Result<(), xproto::ClientMessageEvent> {
        match self.check_state {
            CheckState::WaitForClientNotify => {
                // check to ensure it's ours
                if message.type_ != self.core.atoms[CONNECT_INDEX] {
                    return Err(message);
                }

                self.current_server_mut().accept_window = Some(message.data.as_data32()[0]);
                self.check_state = CheckState::EstablishingConnection;
                Ok(())
            }
            _ => Err(message),
        }
    }

    /// Indicate that we've established a connection.
    pub fn complete_establish(&mut self) {
        match self.check_state {
            CheckState::EstablishingConnection => {
                self.check_state = CheckState::Complete;
            }
            _ => panic!("wasn't expecting connection establishment"),
        }
    }

    fn parse_address(&mut self, address: Vec<u8>) -> Result<(), ConnectError> {
        // split by comma or slash
        let mut splitter = address.split(|c| matches!(c, b'/' | b','));
        let protocol = splitter.next().expect("impossible");
        let host = splitter
            .next()
            .ok_or_else(|| ConnectError::BadProtocol(BadProtocol))?;

        // parse the protocol
        let protocol: Protocol = str::from_utf8(protocol)
            .map_err(|_| ConnectError::BadProtocol(BadProtocol))?
            .parse()
            .map_err(|bp| ConnectError::BadProtocol(bp))?;
        let host = host.to_vec();
        self.current_server_mut().address = Some(Address::new(protocol, host));
        Ok(())
    }

    /// Indicate that we got the property reply.
    pub fn complete_property_reply(&mut self, reply: xproto::GetPropertyReply) {
        match self.check_state {
            CheckState::GetTransport => {
                // parse the name of the transport into an address
                let transport_len = reply.value_len as usize * (reply.format as usize / 8);
                let mut transport_name = reply.value;
                transport_name.truncate(transport_len);

                if let Err(err) = self.parse_address(transport_name) {
                    self.last_err = Some(err);
                    self.early_out();
                    return;
                }

                // move on to the next stage
                self.check_state = CheckState::WindowNotNeeded;
            }
            _ => panic!("wasn't expecting property reply"),
        }
    }

    /// Indicate that we've received a ConnectReply frame.
    pub fn complete_connect_reply_frame(&mut self, _fr: ConnectReplyFr) {
        match self.check_state {
            CheckState::WaitForConnectReply => {
                self.check_state = CheckState::SendOpenFrame;
            }
            _ => panic!("wasn't expecting connect reply frame"),
        }
    }

    /// Indicate that we've received an OpenReply frame.
    pub fn complete_open_reply_frame(&mut self, fr: OpenReplyFr<'_>) {
        match self.check_state {
            CheckState::WaitForOpenReply => {
                self.current_server_mut().connect_id = Some(fr.input_method_id);
                self.check_state = CheckState::Complete;
            }
            _ => panic!("wasn't expecting open reply frame"),
        }
    }

    /// Call the specified completion event.
    pub fn complete(&mut self, completion: Completion) -> Result<(), Event> {
        match completion {
            Completion::AtomCheck(gsor, iar) => {
                self.complete_atom_check(gsor, iar);
            }
            Completion::ClientMessageEvent(cme) => {
                if let Err(cme) = self.complete_client_message(cme) {
                    return Err(Event::ClientMessage(cme));
                }
            }
            Completion::ConnectReply(cfr) => {
                self.complete_connect_reply_frame(cfr);
            }
            Completion::OpenReply(ofr) => {
                self.complete_open_reply_frame(ofr);
            }
            Completion::Property(prop) => {
                self.complete_property_reply(prop);
            }
            Completion::RequestCompleted => {
                self.complete_request();
            }
            Completion::SelectionNotifyEvent(sne) => {
                if let Err(sne) = self.complete_selection_notify(sne) {
                    return Err(Event::SelectionNotify(sne));
                }
            }
            Completion::XidGenerated(xid) => {
                self.complete_xid(xid);
            }
        }

        Ok(())
    }

    /// Convert to the final stage of being a full
    /// instance.
    pub fn build(mut self) -> Result<super::Xim, ConnectError> {
        let error = self.last_err.take();
        self.build_impl()
            .ok_or_else(|| error.unwrap_or(ConnectError::Incomplete))
    }

    fn build_impl(mut self) -> Option<super::Xim> {
        let current_server = self.current_server.take()?;

        if self.check_state != CheckState::Complete {
            return None;
        }

        Some(super::Xim {
            protocol: self.core.atoms[PROTOCOL_INDEX],
            accept: current_server.accept_window?,
            connect_id: current_server.connect_id?,
            client: current_server.client_window?,
            sequence: 2,
        })
    }
}

#[derive(Debug)]
pub enum ConnectError {
    RetrievedServersNotAtoms,
    OutOfServers,
    BadProtocol(BadProtocol),
    NoServerAtom,
    AtomNameMismatch,
    Incomplete,
}

#[cfg(target_endian = "little")]
const BYTE_ORDER: u8 = b'l';
#[cfg(target_endian = "big")]
const BYTE_ORDER: u8 = b'B';
