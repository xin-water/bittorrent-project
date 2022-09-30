//! Library for parsing and converting bencoded data.
//!
//! # Examples
//!
//! Decoding bencoded data:
//!
//! ```rust
//!
//!
//!     use bittorrent_protocol_bencode::{Bencode};
//!
//!     fn main() {
//!         let data = b"d12:lucky_numberi7ee";
//!         let bencode = Bencode::decode(data).unwrap();
//!
//!         assert_eq!(7, bencode.dict().unwrap().lookup("lucky_number".as_bytes())
//!             .unwrap().int().unwrap());
//!     }
//! ```
//!
//! Encoding bencoded data:
//!
//! ```rust
//!
//!     use bittorrent_protocol_bencode::{Bencode};
//!
//!     fn main() {
//!         let message = (ben_map!{
//!             "lucky_number" => ben_int!(7)
//!         }).encode();
//!
//!         assert_eq!(&b"d12:lucky_numberi7ee"[..], &message[..]);
//!     }
//! ```

mod bencode;
mod convert;
mod decode;
mod dictionary;
mod encode;
mod error;

pub use bencode::{Bencode, BencodeKind};
pub use convert::BencodeConvert;
pub use dictionary::Dictionary;
pub use error::{BencodeConvertError, BencodeConvertErrorKind, BencodeConvertResult};
pub use error::{BencodeParseError, BencodeParseErrorKind, BencodeParseResult};

const BEN_END: u8 = b'e';
const DICT_START: u8 = b'd';
const LIST_START: u8 = b'l';
const INT_START: u8 = b'i';

const BYTE_LEN_LOW: u8 = b'0';
const BYTE_LEN_HIGH: u8 = b'9';
const BYTE_LEN_END: u8 = b':';

/// Construct a Bencode map by supplying string references as keys and Bencode as values.
macro_rules! dht_ben_map {
( $($key:expr => $val:expr),* ) => {
        {
            use std::convert::{AsRef};
            use std::collections::{BTreeMap};
            use crate::bencode::{Bencode};

            let mut map = BTreeMap::new();
            $(
                map.insert(AsRef::as_ref($key), $val);
            )*
            Bencode::Dict(map)
        }
    }
}

/// Construct a Bencode list by supplying a list of Bencode values.
macro_rules! dht_ben_list {
    ( $($ben:expr),* ) => {
        {
            use crate::bencode::{Bencode};

            let mut list = Vec::new();
            $(
                list.push($ben);
            )*
            Bencode::List(list)
        }
    }
}

/// Construct Bencode bytes by supplying a type convertible to Vec\<u8\>.
macro_rules! dht_ben_bytes {
    ( $ben:expr ) => {{
        use crate::bencode::Bencode;
        use std::convert::AsRef;

        Bencode::Bytes(AsRef::as_ref($ben))
    }};
}

/// Construct a Bencode integer by supplying an i64.
macro_rules! dht_ben_int {
    ( $ben:expr ) => {{
        use crate::bencode::Bencode;

        Bencode::Int($ben)
    }};
}
