// MIT/Apache2 License

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub(crate) mod frame;

#[rustfmt::skip]
#[path = "automatically_generated.rs"]
pub mod protocol;
pub mod connection;
pub mod message;

pub(crate) fn roundup(val: usize, factor: usize) -> usize {
    (val + factor - 1) / factor * factor
}

#[cfg(test)]
mod tests {
    use super::roundup;

    #[test]
    fn roundup_sanity() {
        assert_eq!(roundup(3, 4), 4);
        assert_eq!(roundup(4, 4), 4);
        assert_eq!(roundup(5, 4), 8);
        assert_eq!(roundup(256, 4), 256);
        assert_eq!(roundup(257, 4), 260);
    }
}
