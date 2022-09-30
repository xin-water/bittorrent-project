use std::io::{self, Write};

use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use nom::{be_u32, be_u8, ErrorKind, IResult};

use btp_bencode::{BConvert, BDecodeOpt, BencodeRef};
use crate::message::{self, bencode, bits_ext, ExtendedMessage, ExtendedType, PeerWireProtocolMessage, MESSAGE_LENGTH_LEN_BYTES, u32_to_usize};

const EXTENSION_HEADER_LEN: usize = message::HEADER_LEN + 1;

mod ut_metadata;
pub use self::ut_metadata::{
    UtMetadataDataMessage, UtMetadataMessage, UtMetadataRejectMessage, UtMetadataRequestMessage,
};

mod null;
pub use self::null::NullProtocolMessage;

/// Enumeration of `BEP 10` extension protocol compatible messages.
#[derive(Debug,PartialEq)]
pub enum PeerExtensionProtocolMessage
{
    UtMetadata(UtMetadataMessage),
    //UtPex(UtPexMessage),
    Custom(NullProtocolMessage),
}

impl PeerExtensionProtocolMessage {

    pub fn bytes_needed(bytes: &[u8]) -> io::Result<Option<usize>> {
        // Follows same length prefix logic as our normal wire protocol...
        match be_u32(bytes) {
            // We need 4 bytes for the length, plus whatever the length is...
            IResult::Done(_, length) => Ok(Some(MESSAGE_LENGTH_LEN_BYTES + u32_to_usize(length))),
            _ => Ok(None),
        }
    }

    pub fn parse_bytes(
        bytes: Bytes,
        extended: &Option<ExtendedMessage>,
    ) -> io::Result<PeerExtensionProtocolMessage> {

        match  extended {
            Some(ref extended_msg) =>{
                match parse_extensions(bytes, extended_msg) {
                    IResult::Done(_, result) => result,
                    _ => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Failed To Parse PeerExtensionProtocolMessage",
                    )),
                }
           }
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                "Extension Message Received From Peer Before Extended Message...",
            ))
        }
    }

    pub fn write_bytes<W>(
        &self,
        mut writer: W,
        extended: &Option<ExtendedMessage>,
    ) -> io::Result<()>
    where
        W: Write,
    {
        match (self,extended) {
            (&PeerExtensionProtocolMessage::UtMetadata(ref msg),Some(ref extended_msg))=> {
                        let ext_id = if let Some(ext_id) = extended_msg.query_id(&ExtendedType::UtMetadata) {
                            ext_id
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                "Can't Send UtMetadataMessage As We Have No Id Mapping",
                            ));
                        };

                        let total_len = (2 + msg.message_size()) as u32;

                        message::write_length_id_pair(
                            &mut writer,
                            total_len,
                            Some(bits_ext::EXTENDED_MESSAGE_ID),
                        )?;
                        writer.write_u8(ext_id)?;

                        msg.write_bytes(writer)
                    }
            (&PeerExtensionProtocolMessage::UtMetadata(ref msg),None)  => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Extension Message Sent From Us Before Extended Message...",
                    )),

            (&PeerExtensionProtocolMessage::Custom(ref msg), _) => msg.write_bytes( writer),
        }
    }

    pub fn message_size(&self ) -> usize {
        match self {
            &PeerExtensionProtocolMessage::UtMetadata(ref msg) => msg.message_size(),
            &PeerExtensionProtocolMessage::Custom(ref msg) => msg.message_size(),
        }
    }
}

fn parse_extensions(
    mut bytes: Bytes,
    extended_msg: &ExtendedMessage,
) -> IResult<(), io::Result<PeerExtensionProtocolMessage>>
{
    let header_bytes = bytes.clone();

    // Attempt to parse a built in message type, otherwise, see if it is an extension type.
    alt!(
        (),
        ignore_input!(
            switch!(header_bytes.as_ref(), throwaway_input!(tuple!(be_u32, be_u8, be_u8)),
                (message_len, bits_ext::EXTENDED_MESSAGE_ID, message_id) =>
                    call!(parse_extensions_with_id, bytes, extended_msg, message_len, message_id)
            )
        )
    )
}

fn parse_extensions_with_id(
    _input: (),
    mut bytes: Bytes,
    extended_msg: &ExtendedMessage,
    message_len:u32,
    message_id: u8,
) -> IResult<(), io::Result<PeerExtensionProtocolMessage>>
{
    let msg_len= message_len as usize - 2;

    let mut temp_bytes = bytes.split_off(EXTENSION_HEADER_LEN);

    if temp_bytes.len() < msg_len {

        let result =  Err(io::Error::new(
            io::ErrorKind::Other,
            format!("PeerExtensionProtocolMessage temp_bytes len < : {:?}", msg_len),
        ));

       return  IResult::Done((), result);
    }

    let msg_bytes= temp_bytes.split_to(msg_len);

    let lt_metadata_id = extended_msg.query_id(&ExtendedType::UtMetadata);
    //let ut_pex_id = extended.query_id(&ExtendedType::UtPex);

    let result = if lt_metadata_id == Some(message_id) {
        UtMetadataMessage::parse_bytes(msg_bytes)
            .map(|lt_metadata_msg| PeerExtensionProtocolMessage::UtMetadata(lt_metadata_msg))
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Unknown Id For PeerExtensionProtocolMessage: {:?}", message_id),
        ))
    };

    IResult::Done((), result)

}
