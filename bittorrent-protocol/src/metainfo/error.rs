//! Errors for torrent file building and parsing.

use std::io;

use crate::bencode::{BencodeConvertError, BencodeParseError};
use walkdir;

error_chain! {
    types {
        ParseError, ParseErrorKind, ParseResultEx, ParseResult;
    }

    foreign_links {
        Io(io::Error);
        Dir(walkdir::Error);
        BencodeConvert(BencodeConvertError);
        BencodeParse(BencodeParseError);
    }

    errors {
        MissingData {
            details: String
        } {
            description("Missing Data Detected In File")
            display("Missing Data Detected In File: {}", details)
        }
    }
}
