// MIT/Apache2 License

use alloc::vec::Vec;
use bytemuck::{bytes_of, pod_read_unaligned, Pod};
use core::{fmt, mem::size_of};

#[cfg(test)]
use bytemuck::AnyBitPattern;

/// A frame that can be sent over the wire.
pub(crate) trait Frame: Sized {
    /// Parse this frame from a byte stream.
    fn parse(bytes: &[u8]) -> Result<WithRemainder<'_, Self>, ParseError>;
    /// The size of this frame in bytes.
    fn size(&self) -> usize;
    /// Serialize this frame to a byte stream.
    fn serialize(&self, target: &mut [u8]) -> usize;
    /// Convert this `Frame` to a `Vec<u8>`.
    fn into_bytes(self) -> Vec<u8> {
        let mut bytes = alloc::vec![0; self.size()];
        self.serialize(&mut bytes);
        bytes
    }
}

/// Generate a random element of this type.
#[cfg(test)]
pub(crate) trait GenRandom: Sized {
    /// Generate a random element of this type.
    fn generate() -> Self;
}

pub(crate) type WithRemainder<'a, T> = (T, &'a [u8]);

#[derive(Debug)]
pub struct ParseError {
    pub(crate) expected: usize,
    pub(crate) got: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expected {} bytes, got {}", self.expected, self.got)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

impl<T: Pod> Frame for T {
    fn parse(bytes: &[u8]) -> Result<WithRemainder<'_, Self>, ParseError> {
        if bytes.len() < size_of::<Self>() {
            return Err(ParseError {
                expected: size_of::<Self>(),
                got: bytes.len(),
            });
        }

        let (data, rem) = bytes.split_at(size_of::<Self>());

        Ok((pod_read_unaligned(data), rem))
    }

    fn size(&self) -> usize {
        size_of::<Self>()
    }

    fn serialize(&self, target: &mut [u8]) -> usize {
        let sz = size_of::<Self>();
        let bytes = bytes_of(self);
        target[..sz].copy_from_slice(bytes);
        sz
    }
}

#[cfg(test)]
impl GenRandom for u8 {
    fn generate() -> Self {
        fastrand::u8(..)
    }
}

#[cfg(test)]
impl GenRandom for u16 {
    fn generate() -> Self {
        fastrand::u16(..)
    }
}

#[cfg(test)]
impl GenRandom for u32 {
    fn generate() -> Self {
        fastrand::u32(..)
    }
}
