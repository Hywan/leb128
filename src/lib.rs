//! Read and write DWARF's "Little Endian Base 128" (LEB128) variable length
//! integer encoding.
//!
//! The implementation is a direct translation of the psuedocode in the DWARF 4
//! standard's appendix C.
//!
//! Read and write signed integers:
//!
//! ```
//! use leb128;
//!
//! let mut buf = [0; 1024];
//!
//! // Write to anything that implements `std::io::Write`.
//! {
//!     let mut writable = &mut buf[..];
//!     leb128::write::signed(&mut writable, -12345).expect("Should write number");
//! }
//!
//! // Read from anything that implements `std::io::Read`.
//! let mut readable = &buf[..];
//! let val = leb128::read::signed(&mut readable).expect("Should read number");
//! assert_eq!(val, -12345);
//! ```
//!
//! Or read and write unsigned integers:
//!
//! ```
//! use leb128;
//!
//! let mut buf = [0; 1024];
//!
//! {
//!     let mut writable = &mut buf[..];
//!     leb128::write::unsigned(&mut writable, 98765).expect("Should write number");
//! }
//!
//! let mut readable = &buf[..];
//! let val = leb128::read::unsigned(&mut readable).expect("Should read number");
//! assert_eq!(val, 98765);
//! ```

#![deny(missing_docs)]

#[doc(hidden)]
pub const CONTINUATION_BIT: u8 = 1 << 7;
#[doc(hidden)]
pub const SIGN_BIT: u8 = 1 << 6;

#[doc(hidden)]
pub fn low_bits_of_byte(byte: u8) -> u8 {
    byte & !CONTINUATION_BIT
}

#[doc(hidden)]
pub fn low_bits_of_u64(val: u64) -> u8 {
    let byte = val & (std::u8::MAX as u64);
    low_bits_of_byte(byte as u8)
}

/// A module for reading signed and unsigned integers that have been LEB128
/// encoded.
pub mod read {
    use super::{CONTINUATION_BIT, SIGN_BIT, low_bits_of_byte};
    use std::fmt;
    use std::io;

    /// An enumeration of the possible errors that can occur when reading a
    /// number encoded with LEB128.
    #[derive(Debug)]
    pub enum Error {
        /// There was an underlying IO error.
        IoError(io::Error),
        /// We were not done reading the number, but there is no more data.
        UnexpectedEndOfData,
        /// The number being read is larger than can be represented.
        Overflow,
    }

    impl From<io::Error> for Error {
        fn from(e: io::Error) -> Self {
            Error::IoError(e)
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
            write!(f,
                   "leb128::read::Error: {}",
                   ::std::error::Error::description(self))
        }
    }

    impl ::std::error::Error for Error {
        fn description(&self) -> &str {
            match *self {
                Error::IoError(ref e) => e.description(),
                Error::UnexpectedEndOfData => "Unexpected end of data while reading",
                Error::Overflow => "The number being read is larger than can be represented",
            }
        }

        fn cause(&self) -> Option<&::std::error::Error> {
            match *self {
                Error::IoError(ref e) => Some(e),
                Error::UnexpectedEndOfData |
                Error::Overflow => None,
            }
        }
    }

    /// Read an unsigned LEB128 number from the given `std::io::Read`able and
    /// return it or an error if reading failed.
    pub fn unsigned<R>(r: &mut R) -> Result<u64, Error>
        where R: io::Read
    {
        let mut result = 0;
        let mut shift = 0;

        loop {
            let mut buf = [0];
            if try!(r.read(&mut buf)) != 1 {
                return Err(Error::UnexpectedEndOfData);
            }

            let low_bits = low_bits_of_byte(buf[0]) as u64;
            if low_bits.leading_zeros() < shift {
                return Err(Error::Overflow);
            }

            result |= low_bits << shift;

            if buf[0] & CONTINUATION_BIT == 0 {
                return Ok(result);
            }

            shift += 7;
        }
    }

    /// Read a signed LEB128 number from the given `std::io::Read`able and
    /// return it or an error if reading failed.
    pub fn signed<R>(r: &mut R) -> Result<i64, Error>
        where R: io::Read
    {
        let mut result = 0;
        let mut shift = 0;
        let size = 64;
        let mut byte;

        loop {
            let mut buf = [0];
            if try!(r.read(&mut buf)) != 1 {
                return Err(Error::UnexpectedEndOfData);
            }

            byte = buf[0];
            let low_bits = low_bits_of_byte(byte) as i64;
            if low_bits.leading_zeros() < shift {
                return Err(Error::Overflow);
            }

            result |= low_bits << shift;
            shift += 7;

            if byte & CONTINUATION_BIT == 0 {
                break;
            }
        }

        if shift < size && (SIGN_BIT & byte) == SIGN_BIT {
            // Sign extend the result.
            result |= -(1 << shift);
        }

        Ok(result)
    }
}

/// A module for writing integers encoded as LEB128.
pub mod write {
    use super::{CONTINUATION_BIT, SIGN_BIT, low_bits_of_u64};
    use std::io;

    /// Write the given unsigned number using the LEB128 encoding to the given
    /// `std::io::Write`able. Returns the number of bytes written to `w`, or an
    /// error if writing failed.
    pub fn unsigned<W>(w: &mut W, mut val: u64) -> Result<usize, io::Error>
        where W: io::Write
    {
        let mut bytes_written = 0;
        loop {
            let mut byte = low_bits_of_u64(val);
            val >>= 7;
            if val != 0 {
                // More bytes to come, so set the continuation bit.
                byte |= CONTINUATION_BIT;
            }

            let buf = [byte];
            bytes_written += try!(w.write(&buf));

            if val == 0 {
                return Ok(bytes_written);
            }
        }
    }

    /// Write the given signed number using the LEB128 encoding to the given
    /// `std::io::Write`able. Returns the number of bytes written to `w`, or an
    /// error if writing failed.
    pub fn signed<W>(w: &mut W, mut val: i64) -> Result<usize, io::Error>
        where W: io::Write
    {
        let mut more = true;
        let mut bytes_written = 0;

        while more {
            let mut byte = (val as u64 & !(CONTINUATION_BIT as u64)) as u8;
            val >>= 7;

            if (val == 0 && (byte & SIGN_BIT) == 0) ||
               (val == -1 && (byte & SIGN_BIT) == SIGN_BIT) {
                more = false;
            } else {
                // More bytes to come, so set the continuation bit.
                byte |= CONTINUATION_BIT;
            }

            let buf = [byte];
            bytes_written += try!(w.write(&buf));
        }

        Ok(bytes_written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_low_bits_of_byte() {
        for i in 0..127 {
            assert_eq!(i, low_bits_of_byte(i));
            assert_eq!(i, low_bits_of_byte(i | CONTINUATION_BIT));
        }
    }

    #[test]
    fn test_low_bits_of_u64() {
        for i in 0u64..127 {
            assert_eq!(i as u8, low_bits_of_u64(1 << 16 | i));
            assert_eq!(i as u8,
                       low_bits_of_u64(i << 16 | i | (CONTINUATION_BIT as u64)));
        }
    }

    // Examples from the DWARF 4 standard, section 7.6, figure 22.
    #[test]
    fn test_read_unsigned() {
        let buf = [2u8];
        let mut readable = &buf[..];
        assert_eq!(2,
                   read::unsigned(&mut readable).expect("Should read number"));

        let buf = [127u8];
        let mut readable = &buf[..];
        assert_eq!(127,
                   read::unsigned(&mut readable).expect("Should read number"));

        let buf = [CONTINUATION_BIT, 1];
        let mut readable = &buf[..];
        assert_eq!(128,
                   read::unsigned(&mut readable).expect("Should read number"));

        let buf = [1u8 | CONTINUATION_BIT, 1];
        let mut readable = &buf[..];
        assert_eq!(129,
                   read::unsigned(&mut readable).expect("Should read number"));

        let buf = [2u8 | CONTINUATION_BIT, 1];
        let mut readable = &buf[..];
        assert_eq!(130,
                   read::unsigned(&mut readable).expect("Should read number"));

        let buf = [57u8 | CONTINUATION_BIT, 100];
        let mut readable = &buf[..];
        assert_eq!(12857,
                   read::unsigned(&mut readable).expect("Should read number"));
    }

    // Examples from the DWARF 4 standard, section 7.6, figure 23.
    #[test]
    fn test_read_signed() {
        let buf = [2u8];
        let mut readable = &buf[..];
        assert_eq!(2, read::signed(&mut readable).expect("Should read number"));

        let buf = [0x7eu8];
        let mut readable = &buf[..];
        assert_eq!(-2, read::signed(&mut readable).expect("Should read number"));

        let buf = [127u8 | CONTINUATION_BIT, 0];
        let mut readable = &buf[..];
        assert_eq!(127,
                   read::signed(&mut readable).expect("Should read number"));

        let buf = [1u8 | CONTINUATION_BIT, 0x7f];
        let mut readable = &buf[..];
        assert_eq!(-127,
                   read::signed(&mut readable).expect("Should read number"));

        let buf = [CONTINUATION_BIT, 1];
        let mut readable = &buf[..];
        assert_eq!(128,
                   read::signed(&mut readable).expect("Should read number"));

        let buf = [CONTINUATION_BIT, 0x7f];
        let mut readable = &buf[..];
        assert_eq!(-128,
                   read::signed(&mut readable).expect("Should read number"));

        let buf = [1u8 | CONTINUATION_BIT, 1];
        let mut readable = &buf[..];
        assert_eq!(129,
                   read::signed(&mut readable).expect("Should read number"));

        let buf = [0x7fu8 | CONTINUATION_BIT, 0x7e];
        let mut readable = &buf[..];
        assert_eq!(-129,
                   read::signed(&mut readable).expect("Should read number"));
    }

    #[test]
    fn test_read_unsigned_not_enough_data() {
        let buf = [CONTINUATION_BIT];
        let mut readable = &buf[..];
        assert!(match read::unsigned(&mut readable) {
            Err(read::Error::UnexpectedEndOfData) => true,
            otherwise => {
                println!("Unexpected: {:?}", otherwise);
                false
            }
        });
    }

    #[test]
    fn test_read_signed_not_enough_data() {
        let buf = [CONTINUATION_BIT];
        let mut readable = &buf[..];
        assert!(match read::signed(&mut readable) {
            Err(read::Error::UnexpectedEndOfData) => true,
            otherwise => {
                println!("Unexpected: {:?}", otherwise);
                false
            }
        });
    }

    #[test]
    fn dogfood_signed() {
        for i in -513..513 {
            let mut buf = [0u8; 1024];

            {
                let mut writable = &mut buf[..];
                write::signed(&mut writable, i).expect("Should write signed number");
            }

            let mut readable = &buf[..];
            let result = read::signed(&mut readable).expect("Should be able to read it back again");
            assert_eq!(i, result);
        }
    }

    #[test]
    fn dogfood_unsigned() {
        for i in 0..1025 {
            let mut buf = [0u8; 1024];

            {
                let mut writable = &mut buf[..];
                write::unsigned(&mut writable, i).expect("Should write signed number");
            }

            let mut readable = &buf[..];
            let result = read::unsigned(&mut readable)
                .expect("Should be able to read it back again");
            assert_eq!(i, result);
        }
    }

    #[test]
    fn test_read_unsigned_overflow() {
        let buf = [2u8 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   1];
        let mut readable = &buf[..];
        assert!(read::unsigned(&mut readable).is_err());
    }

    #[test]
    fn test_read_signed_overflow() {
        let buf = [2u8 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   2 | CONTINUATION_BIT,
                   1];
        let mut readable = &buf[..];
        assert!(read::signed(&mut readable).is_err());
    }

    #[test]
    fn test_read_multiple() {
        let buf = [2u8 | CONTINUATION_BIT, 1u8, 1u8];

        let mut readable = &buf[..];
        assert_eq!(read::unsigned(&mut readable).expect("Should read first number"),
                   130u64);
        assert_eq!(read::unsigned(&mut readable).expect("Should read first number"),
                   1u64);
    }
}
