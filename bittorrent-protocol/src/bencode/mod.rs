//! Library for parsing and converting bencoded data.
//!
//! # Examples
//!
//! Decoding bencoded data:
//!
//! ```rust
//!
//!     use std::default::Default;
//!     use bittorrent_protocol::bencode::{BencodeRef, BRefAccess, BDecodeOpt};
//!
//!     fn main() {
//!         let data = b"d12:lucky_numberi7ee";
//!         let bencode = BencodeRef::decode(data, BDecodeOpt::default()).unwrap();
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
//!     fn main() {
//!         let message = (ben_map!{
//!             "lucky_number" => ben_int!(7),
//!             "lucky_string" => ben_bytes!("7")
//!         }).encode();
//!
//!         assert_eq!(&b"d12:lucky_numberi7e12:lucky_string1:7e"[..], &message[..]);
//!     }
//! ```

mod access;
pub use access::bencode::{BMutAccess, BRefAccess, BencodeMutKind, BencodeRefKind};
pub use access::convert::BConvert;
pub use access::dict::BDictAccess;
pub use access::list::BListAccess;

mod cow;

mod mutable;
pub use mutable::bencode_mut::BencodeMut;

mod reference;
pub use reference::bencode_ref::BencodeRef;
pub use reference::decode_opt::BDecodeOpt;

mod error;
pub use error::{BencodeConvertError, BencodeConvertErrorKind, BencodeConvertResult};
pub use error::{BencodeParseError, BencodeParseErrorKind, BencodeParseResult};

/// Traits for implementation functionality.
pub mod inner {
    pub use super::cow::BCowConvert;
}

/// Traits for extended functionality.
pub mod ext {
    pub use super::access::bencode::BRefAccessExt;
    pub use super::access::convert::BConvertExt;
}

const BEN_END: u8 = b'e';
const DICT_START: u8 = b'd';
const LIST_START: u8 = b'l';
const INT_START: u8 = b'i';

const BYTE_LEN_LOW: u8 = b'0';
const BYTE_LEN_HIGH: u8 = b'9';
const BYTE_LEN_END: u8 = b':';

/// Construct a `BencodeMut` map by supplying string references as keys and `BencodeMut` as values.
#[macro_export]
macro_rules!  ben_map {
( $($key:expr => $val:expr),* ) => {
        {
            use bittorrent_protocol::bencode::{BMutAccess, BencodeMut};
            use bittorrent_protocol::bencode::inner::BCowConvert;

            let mut bencode_map = BencodeMut::new_dict();
            {
                let map = bencode_map.dict_mut().unwrap();
                $(
                    map.insert(BCowConvert::convert($key), $val);
                )*
            }

            bencode_map
        }
    }
}

macro_rules!  bt_ben_map {
( $($key:expr => $val:expr),* ) => {
        {
            use crate::bencode::{BMutAccess, BencodeMut};
            use crate::bencode::inner::BCowConvert;

            let mut bencode_map = BencodeMut::new_dict();
            {
                let map = bencode_map.dict_mut().unwrap();
                $(
                    map.insert(BCowConvert::convert($key), $val);
                )*
            }

            bencode_map
        }
    }
}

/// Construct a `BencodeMut` list by supplying a list of `BencodeMut` values.
#[macro_export]
macro_rules! ben_list {
    ( $($ben:expr),* ) => {
        {
            use bittorrent_protocol::bencode::{BencodeMut, BMutAccess};

            let mut bencode_list = BencodeMut::new_list();
            {
                let list = bencode_list.list_mut().unwrap();
                $(
                    list.push($ben);
                )*
            }

            bencode_list
        }
    }
}

macro_rules! bt_ben_list {
    ( $($ben:expr),* ) => {
        {
            use crate::bencode::{BencodeMut, BMutAccess};

            let mut bencode_list = BencodeMut::new_list();
            {
                let list = bencode_list.list_mut().unwrap();
                $(
                    list.push($ben);
                )*
            }

            bencode_list
        }
    }
}

/// Construct `BencodeMut` bytes by supplying a type convertible to `Vec<u8>`.
#[macro_export]
macro_rules! ben_bytes {
    ( $ben:expr ) => {{
        use bittorrent_protocol::bencode::inner::BCowConvert;
        use bittorrent_protocol::bencode::BencodeMut;

        BencodeMut::new_bytes(BCowConvert::convert($ben))
    }};
}

macro_rules! bt_ben_bytes {
    ( $ben:expr ) => {{
        use crate::bencode::inner::BCowConvert;
        use crate::bencode::BencodeMut;

        BencodeMut::new_bytes(BCowConvert::convert($ben))
    }};
}

/// Construct a `BencodeMut` integer by supplying an `i64`.
#[macro_export]
macro_rules! ben_int {
    ( $ben:expr ) => {{
        use bittorrent_protocol::bencode::BencodeMut;

        BencodeMut::new_int($ben)
    }};
}

macro_rules! bt_ben_int {
    ( $ben:expr ) => {{
        use crate::bencode::BencodeMut;

        BencodeMut::new_int($ben)
    }};
}
