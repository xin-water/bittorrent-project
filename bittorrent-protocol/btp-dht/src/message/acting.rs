use btp_util::bt::NodeId;
use btp_bencode::{BConvert, BDictAccess, BencodeRef};
use crate::error::DhtResult;
use crate::message;
use crate::message::request;
use crate::message::request::RequestValidate;

// Ping与AnnouncePeer响应类型完全一致，故在不想用事务id判断类型时直接使用acting类型代替。
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ActingResponse<'a> {
      trans_id: &'a [u8],
      node_id: NodeId,
}


impl<'a> ActingResponse<'a> {
     pub fn new(trans_id: &'a [u8], node_id: NodeId) -> ActingResponse<'a> {
          ActingResponse {
               trans_id: trans_id,
               node_id: node_id,
          }
     }

     pub fn from_parts(
          rqst_root: &dyn BDictAccess<&[u8], BencodeRef<'a>>,
          trans_id: &'a [u8],
     ) -> DhtResult<ActingResponse<'a>> {
          let validate = RequestValidate::new(&trans_id);

          let node_id_bytes = validate.lookup_and_convert_bytes(rqst_root, message::NODE_ID_KEY)?;
          let node_id = validate.validate_node_id(node_id_bytes)?;

          Ok(ActingResponse::new(trans_id, node_id))
     }

     pub fn transaction_id(&self) -> &'a [u8] {
          self.trans_id
     }

     pub fn node_id(&self) -> NodeId {
          self.node_id
     }

     pub fn encode(&self) -> Vec<u8> {
          (ben_map!{
            //message::CLIENT_TYPE_KEY => ben_bytes!(dht::CLIENT_IDENTIFICATION),
            message::TRANSACTION_ID_KEY => ben_bytes!(self.trans_id),
            message::MESSAGE_TYPE_KEY => ben_bytes!(message::RESPONSE_TYPE_KEY),
            message::RESPONSE_TYPE_KEY => ben_map!{
                message::NODE_ID_KEY => ben_bytes!(self.node_id.as_ref())
            }
        })
              .encode()
     }
}
