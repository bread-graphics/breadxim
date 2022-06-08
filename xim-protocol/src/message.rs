// MIT/Apache2 License

use core::fmt::Display;

use crate::{frame::Frame, protocol::XimFrame};
use alloc::{
    borrow::Cow,
    string::{String, ToString},
    vec::Vec,
};
use core::{cmp, fmt, mem};
use x11rb_protocol::protocol::xproto::{
    self, Atom, ChangePropertyRequest, ClientMessageEvent, GetPropertyReply, GetPropertyRequest,
    InternAtomRequest,
};

const INLINE_THRESHOLD: usize = 20;

/// The serialized form of an `XimFrame`.
///
/// This is intended to be either sent or received from the client using
/// the `ClientMessageEvent` type of transport.
#[derive(Debug)]
pub struct Message {
    // the client message event
    message: ClientMessageEvent,
    // any other information that needs to be sent
    head: Head,
}

enum Head {
    Nothing,
    AwaitingAtom { atom_name: String, data: Vec<u8> },
}

impl fmt::Debug for Head {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Container(usize);

        impl fmt::Debug for Container {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "<{} bytes>", self.0)
            }
        }

        match self {
            Head::Nothing => write!(f, "Nothing"),
            Head::AwaitingAtom { atom_name, data } => f
                .debug_struct("AwaitingAtom")
                .field("atom_name", &atom_name)
                .field("data", &Container(data.len()))
                .finish(),
        }
    }
}

impl Message {
    /// Create a new `Message` from the `XimFrame` type.
    pub fn new<'a>(
        frame: impl Into<XimFrame<'a>>,
        name: impl Display,
        protocol_atom: impl Into<Atom>,
        window: impl Into<xproto::Window>,
    ) -> Self {
        Self::new_impl(frame.into(), &name, protocol_atom.into(), window.into())
    }

    fn new_impl(
        frame: XimFrame<'_>,
        name: &dyn Display,
        atom: Atom,
        window: xproto::Window,
    ) -> Self {
        // serialize the frame into a Vec<u8>
        let data = frame.into_bytes();

        // if we can inline it into a client message, do that
        let (format, data, head) = if data.len() <= INLINE_THRESHOLD {
            // we can!
            let mut client_data = [0u8; 20];
            client_data[..data.len()].copy_from_slice(&data);
            (8, client_data.into(), Head::Nothing)
        } else {
            // we can't, set us up so we can attach the data to the window
            // as a property
            let name = name.to_string();
            // dummy data
            let client_data = [0u8; 20];

            (
                32,
                client_data.into(),
                Head::AwaitingAtom {
                    atom_name: name,
                    data,
                },
            )
        };

        let message = ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT,
            format,
            sequence: 0,
            window,
            type_: atom,
            data,
        };

        Self { message, head }
    }

    /// Initialize this `Message` from an existing `ClientMessageEvent`.
    ///
    /// Note that there may be additional data that needs to be retrieved.
    /// This is done by calling `get_property` on the window. The request
    /// may also be provided by this function.
    ///
    /// # Errors
    ///
    /// Returns an error if the client message is in an invalid format.
    pub fn from_client_message(
        cme: ClientMessageEvent,
    ) -> Result<(Self, Option<GetPropertyRequest>), InvalidFormat> {
        let message = Message {
            message: cme,
            head: Head::Nothing,
        };

        // also determine whether we need the property
        match message.message.format {
            8 => {
                // we don't need it
                Ok((message, None))
            }
            32 => {
                // we do need it
                let data = message.message.data.as_data32();
                let len = data[0];
                let atom = data[1];

                let gpr = GetPropertyRequest {
                    delete: true,
                    window: message.message.window,
                    property: atom,
                    type_: xproto::AtomEnum::ANY.into(),
                    long_offset: 0,
                    long_length: len,
                };
                Ok((message, Some(gpr)))
            }
            format => Err(InvalidFormat(format)),
        }
    }

    /// Get the `InternAtomRequest` that we later expect to feed
    /// back into here.
    ///
    /// Take the atom reply and feed it to the `use_atom` function.
    pub fn intern_atom(&self) -> Option<InternAtomRequest<'_>> {
        match self.head {
            Head::AwaitingAtom { ref atom_name, .. } => Some(InternAtomRequest {
                only_if_exists: false,
                name: Cow::Borrowed(atom_name.as_bytes()),
            }),
            _ => None,
        }
    }

    /// Use the given atom to create a `ChangePropertyRequest` to
    /// populate the window with this message.
    ///
    /// Returns two requests. Fetch/retrieve the GPR, and then use
    /// the CPR.
    ///
    /// # Panics
    ///
    /// Panics if the `Message` was not expecting an atom, since
    /// `intern_atom` returned `None`.
    pub fn use_atom(
        &mut self,
        message_atom: Atom,
    ) -> (GetPropertyRequest, ChangePropertyRequest<'static>) {
        match self.head {
            Head::AwaitingAtom { ref mut data, .. } => {
                // take data
                let data = mem::take(data);
                let data_len = data.len();

                // create a request to get the property
                // this ensures it's created or cleared
                let gpr = GetPropertyRequest {
                    delete: false,
                    window: self.message.window,
                    property: message_atom,
                    type_: xproto::AtomEnum::STRING.into(),
                    long_offset: 0,
                    long_length: 10000,
                };

                // create request
                let cpr = ChangePropertyRequest {
                    mode: xproto::PropMode::APPEND,
                    window: self.message.window,
                    property: message_atom,
                    type_: xproto::AtomEnum::STRING.into(),
                    format: 8,
                    data_len: data_len as _,
                    data: data.into(),
                };

                // replace head now that we're awaiting
                self.head = Head::Nothing;
                let mut msg_data = [0u32; 5];
                msg_data[0] = data_len as _;
                msg_data[1] = message_atom;
                self.message.data = msg_data.into();

                (gpr, cpr)
            }
            _ => panic!("called use_atom() on a message that wasn't expecting an atom"),
        }
    }

    /// Try to convert this item into a `ClientMessageEvent` to be
    /// sent.
    ///
    /// # Panics
    ///
    /// Panics is the atom is not yet resolved, if need be.
    pub fn into_client_message(self) -> ClientMessageEvent {
        self.try_into()
            .unwrap_or_else(|_| panic!("Unable to resolve `Message` to `ClientMessageEvent`"))
    }

    /// Parse the data from a `Message` (and a potential `GetPropertyRequest`)
    /// into a piece of raw data.
    pub fn parse(self, gpr: Option<GetPropertyReply>) -> Vec<u8> {
        match gpr {
            None => {
                // all data is contained entirely within the message
                let msg_data = self.message.data.as_data8();

                // from here, get the length
                let len = (bytemuck::pod_read_unaligned::<u16>(&msg_data[2..4]) as usize + 1) * 4;
                msg_data[..len].to_vec()
            }
            Some(gpr) => {
                let reply_len = (gpr.value_len as usize) * (gpr.format as usize / 8);
                let mut reply_data = gpr.value;

                // from here, get the length
                let len = (bytemuck::pod_read_unaligned::<u16>(&reply_data[2..4]) as usize + 1) * 4;
                let real_len = cmp::min(len, reply_len);
                reply_data.truncate(real_len);
                reply_data
            }
        }
    }
}

impl TryFrom<Message> for ClientMessageEvent {
    type Error = Message;

    fn try_from(value: Message) -> Result<Self, Self::Error> {
        let Message { message, head } = value;

        match head {
            Head::Nothing => Ok(message),
            head => Err(Message { message, head }),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use x11rb_protocol::protocol::xproto::{self, GetPropertyReply};

    use crate::{
        frame::Frame,
        message::Message,
        protocol::{OpenFr, PreeditDoneFr, StrFr, XimFrame},
    };

    #[test]
    fn message_ser_and_deser() {
        fn with_frame(fr: XimFrame) {
            // for comparison
            let data = fr.clone().into_bytes();
            let mut present_in_msg = None;

            // dummy values
            let protocol_atom = fastrand::u32(1..);
            let window = fastrand::u32(1..);
            let message_atom = fastrand::u32(1..);

            // create a new message
            let mut msg = Message::new(fr, "atom_name", protocol_atom, window);

            // pretend like we sent the configure requests if need be
            if msg.intern_atom().is_some() {
                let (gpr, cpr) = msg.use_atom(message_atom);

                // ensure properties re correct
                assert_eq!(gpr.property, message_atom);
                assert_eq!(cpr.property, message_atom);
                assert_eq!(cpr.format, 8);
                assert_eq!(cpr.data_len as usize, data.len());
                assert_eq!(cpr.data, &*data);

                // format as a GetPropertyReply for parsing later
                present_in_msg = Some(GetPropertyReply {
                    format: 8,
                    sequence: 0,
                    length: cpr.data_len,
                    type_: xproto::AtomEnum::STRING.into(),
                    bytes_after: 0,
                    value_len: cpr.data_len,
                    value: cpr.data.into_owned(),
                });
            }

            // convert to client message
            let msg = msg.into_client_message();

            assert_eq!(msg.type_, protocol_atom);
            assert_eq!(msg.window, window);
            if present_in_msg.is_none() {
                assert_eq!(msg.format, 8);
                assert_eq!(&msg.data.as_data8()[..data.len()], &data[..]);
            } else {
                assert_eq!(msg.format, 32);
                assert_eq!(msg.data.as_data32()[0], data.len() as _);
                assert_eq!(msg.data.as_data32()[1], message_atom);
            }

            // convert back to message
            let (msg, gpr) = Message::from_client_message(msg).unwrap();
            let reply = gpr.and(present_in_msg);
            let parsed_data = msg.parse(reply);
            assert_eq!(data, parsed_data);
        }

        // large and small request
        let small = PreeditDoneFr {
            input_context_id: fastrand::u16(1..),
            input_method_id: fastrand::u16(1..),
        };
        let sarray = core::iter::repeat_with(|| fastrand::u8(1..))
            .take(100)
            .collect::<Vec<_>>();
        let large = OpenFr {
            field1: StrFr {
                string: sarray.into(),
            },
        };

        with_frame(small.into());
        with_frame(large.into());
    }
}

#[derive(Debug)]
pub struct InvalidFormat(u8);

impl fmt::Display for InvalidFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid client message format: {}", self.0)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for InvalidFormat {}