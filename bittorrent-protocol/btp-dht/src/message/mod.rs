
use btp_bencode::{BConvert, BencodeConvertError, BencodeRef};
use crate::error::{DhtError, DhtErrorKind, DhtResult};
use crate::message::error::ErrorMessage;
use crate::message::request::RequestType;
use crate::message::response::{ExpectedResponse, ResponseType};

pub mod compact_info;

pub mod error;
pub mod request;
pub mod response;

pub mod announce_peer;
pub mod find_node;
pub mod get_peers;
pub mod ping;
pub(crate) mod acting;

// Top level message keys
const TRANSACTION_ID_KEY: &'static str = "t";
const MESSAGE_TYPE_KEY: &'static str = "y";
// const CLIENT_TYPE_KEY:    &'static str = "v";

// Top level message type sentinels
const REQUEST_TYPE_KEY: &'static str = "q";
const RESPONSE_TYPE_KEY: &'static str = "r";
const ERROR_TYPE_KEY: &'static str = "e";

// Refers to root dictionary itself
const ROOT_ID_KEY: &'static str = "root";

// Keys common across message types
const NODE_ID_KEY: &'static str = "id";
const NODES_KEY: &'static str = "nodes";
const VALUES_KEY: &'static str = "values";
const TARGET_ID_KEY: &'static str = "target";
const INFO_HASH_KEY: &'static str = "info_hash";
const TOKEN_KEY: &'static str = "token";

// ----------------------------------------------------------------------------//

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
struct MessageValidate;

impl BConvert for MessageValidate {
    type Error = DhtError;

    fn handle_error(&self, error: BencodeConvertError) -> DhtError {
        error.into()
    }
}

// ----------------------------------------------------------------------------//

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MessageType<'a> {
    Request(RequestType<'a>),
    Response(ResponseType<'a>),
    Error(ErrorMessage<'a>),
}

impl<'a> MessageType<'a> {
    pub fn new(message: &'a BencodeRef<'a>) -> DhtResult<MessageType<'a>>
    {
        let validate = MessageValidate;
        let msg_root = validate.convert_dict(message, ROOT_ID_KEY)?;

        let trans_id = validate.lookup_and_convert_bytes(msg_root, TRANSACTION_ID_KEY)?;
        let msg_type = validate.lookup_and_convert_str(msg_root, MESSAGE_TYPE_KEY)?;

        match msg_type {
            REQUEST_TYPE_KEY => {
                let rqst_type = validate.lookup_and_convert_str(msg_root, REQUEST_TYPE_KEY)?;
                let rqst_msg = RequestType::from_parts(msg_root, trans_id, rqst_type)?;
                Ok(MessageType::Request(rqst_msg))
            }
            RESPONSE_TYPE_KEY => {
                //let rsp_type = trans_mapper(trans_id);
                let rsp_message = ResponseType::from_parts(msg_root, trans_id)?;
                Ok(MessageType::Response(rsp_message))
            }
            ERROR_TYPE_KEY => {
                let err_message = ErrorMessage::from_parts(msg_root, trans_id)?;
                Ok(MessageType::Error(err_message))
            }
            unknown => Err(DhtError::from_kind(DhtErrorKind::InvalidMessage {
                code: unknown.to_owned(),
            })),
        }
    }
}
