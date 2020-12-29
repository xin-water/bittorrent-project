#![macro_use]
use bencode;

use std::{convert, io};

#[macro_export]
macro_rules! get_field_with_default {
    ($m:expr, $field:expr, $default:expr) => {
        match $m.get(&ByteString::from_str($field)) {
            Some(a) => match FromBencode::from_bencode(a) {
                Result::Err(e) => {
                    eprintln!("{:?}", e);
                    None
                }
                Result::Ok(v) => Some(v),
            },
            None => None,
        }
    };
}

#[macro_export]
macro_rules! get_field {
    ($m:expr, $field:expr) => {
        get_field_with_default!(
            $m,
            $field,
            return Err(decoder::Error::DoesntContain($field))
        )
    };
}

#[macro_export]
macro_rules! get_optional_field {
    ($m:expr, $field:expr) => {
        get_field_with_default!($m, $field, None)
    };
}

#[macro_export]
macro_rules! get_raw_field {
    ($m:expr, $field:expr) => {
        $m.get(&ByteString::from_str($field))
    };
}

#[macro_export]
macro_rules! get_field_as_bencoded_bytes {
    ($m:expr, $field:expr) => {
        get_raw_field!($m, $field).unwrap().to_bytes().unwrap()
    };
}

#[macro_export]
macro_rules! get_field_as_bytes {
    ($m:expr, $field:expr) => {
        match get_raw_field!($m, $field).unwrap() {
            &Bencode::ByteString(ref v) => v.clone(),
            _ => return Err(decoder::Error::NotAByteString),
        }
    };
}

#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    DecodingError(bencode::streaming::Error),
    NotADict,
    NotAByteString,
    DoesntContain(&'static str),
    NotANumber(bencode::NumFromBencodeError),
    NotAString(bencode::StringFromBencodeError),
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl convert::From<bencode::streaming::Error> for Error {
    fn from(err: bencode::streaming::Error) -> Error {
        Error::DecodingError(err)
    }
}

impl convert::From<bencode::NumFromBencodeError> for Error {
    fn from(err: bencode::NumFromBencodeError) -> Error {
        Error::NotANumber(err)
    }
}

impl convert::From<bencode::StringFromBencodeError> for Error {
    fn from(err: bencode::StringFromBencodeError) -> Error {
        Error::NotAString(err)
    }
}
