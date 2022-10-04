// use crate::bencode::{Bencode, BencodeConvert, Dictionary, BencodeConvertError};
use btp_util::bt::NodeId;

use crate::bencode::{Bencode, BencodeConvert, BencodeConvertError, Dictionary};
use crate::error::{DhtError, DhtErrorKind, DhtResult};
use crate::message::acting::ActingResponse;
use crate::message::announce_peer::AnnouncePeerResponse;
use crate::message::compact_info::{CompactNodeInfo, CompactValueInfo};
use crate::message::find_node::FindNodeResponse;
use crate::message::get_peers::GetPeersResponse;
use crate::message::ping::PingResponse;

use crate::message::RESPONSE_TYPE_KEY;

use crate::message::TOKEN_KEY;
use crate::message::NODES_KEY;

pub const RESPONSE_ARGS_KEY: &'static str = "r";

// ----------------------------------------------------------------------------//

pub struct ResponseValidate<'a> {
    trans_id: &'a [u8],
}

impl<'a> ResponseValidate<'a> {
    pub fn new(trans_id: &'a [u8]) -> ResponseValidate<'a> {
        ResponseValidate { trans_id: trans_id }
    }

    pub fn validate_node_id(&self, node_id: &[u8]) -> DhtResult<NodeId> {
        NodeId::from_hash(node_id).map_err(|_| {
            DhtError::from_kind(DhtErrorKind::InvalidResponse {
                details: format!(
                    "TID {:?} Found Node ID With Invalid Length {:?}",
                    self.trans_id,
                    node_id.len()
                ),
            })
        })
    }

    /// Validate the given nodes string which should be IPv4 compact
    pub fn validate_nodes<'b>(&self, nodes: &'b [u8]) -> DhtResult<CompactNodeInfo<'b>> {
        CompactNodeInfo::new(nodes).map_err(|_| {
            DhtError::from_kind(DhtErrorKind::InvalidResponse {
                details: format!(
                    "TID {:?} Found Nodes Structure With {} Number Of Bytes Instead \
                                  Of Correct Multiple",
                    self.trans_id,
                    nodes.len()
                ),
            })
        })
    }

    pub fn validate_values<'b>(
        &self,
        values: &'b [Bencode<'a>],
    ) -> DhtResult<CompactValueInfo<'b>> {
        for bencode in values.iter() {
            match bencode.bytes() {
                Some(_) => (),
                None => {
                    return Err(DhtError::from_kind(DhtErrorKind::InvalidResponse {
                        details: format!(
                            "TID {:?} Found Values Structure As Non Bytes Type",
                            self.trans_id
                        ),
                    }))
                }
            }
        }

        CompactValueInfo::new(values).map_err(|_| {
            DhtError::from_kind(DhtErrorKind::InvalidResponse {
                details: format!(
                    "TID {:?} Found Values Structrue With Wrong Number Of Bytes",
                    self.trans_id
                ),
            })
        })
    }
}

impl<'a> BencodeConvert for ResponseValidate<'a> {
    type Error = DhtError;

    fn handle_error(&self, error: BencodeConvertError) -> DhtError {
        error.into()
    }
}

// ----------------------------------------------------------------------------//

//Ping与AnnouncePeer响应类型完全一致，故在不想用事务id判断类型时直接使用acting类型代替。

#[allow(unused)]
pub enum ExpectedResponse {
    //Ping,
    FindNode,
    GetPeers,
    //AnnouncePeer,
    Acting,
    GetData,
    PutData,
    None,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResponseType<'a> {
    Ping(PingResponse<'a>),
    FindNode(FindNodeResponse<'a>),
    GetPeers(GetPeersResponse<'a>),
    AnnouncePeer(AnnouncePeerResponse<'a>),
    Acting(ActingResponse<'a>),

    /* GetData(GetDataResponse<'a>),
     * PutData(PutDataResponse<'a>) */
}

impl<'a> ResponseType<'a> {
    pub fn from_parts(
        root: &'a dyn Dictionary<'a, Bencode<'a>>,
        trans_id: &'a [u8],
        //rsp_type: ExpectedResponse,
    ) -> DhtResult<ResponseType<'a>> {
        let validate = ResponseValidate::new(trans_id);
        let rqst_root = validate.lookup_and_convert_dict(root, RESPONSE_ARGS_KEY)?;
        let rsp_type={

            if rqst_root.lookup(TOKEN_KEY.as_bytes()).is_some(){
                ExpectedResponse::GetPeers
            }else if  rqst_root.lookup(NODES_KEY.as_bytes()).is_some(){
                ExpectedResponse::FindNode
            }else {
                ExpectedResponse::Acting
            }

        } ;



        match rsp_type {
            // ExpectedResponse::Ping => {
            //     let ping_rsp = PingResponse::from_parts(rqst_root, trans_id)?;
            //     Ok(ResponseType::Ping(ping_rsp))
            // }
            ExpectedResponse::FindNode => {
                let find_node_rsp = FindNodeResponse::from_parts(rqst_root, trans_id)?;
                Ok(ResponseType::FindNode(find_node_rsp))
            }
            ExpectedResponse::GetPeers => {
                let get_peers_rsp = GetPeersResponse::from_parts(rqst_root, trans_id)?;
                Ok(ResponseType::GetPeers(get_peers_rsp))
            }
            // ExpectedResponse::AnnouncePeer => {
            //     let announce_peer_rsp = AnnouncePeerResponse::from_parts(rqst_root, trans_id)?;
            //     Ok(ResponseType::AnnouncePeer(announce_peer_rsp))
            // }

            ExpectedResponse::Acting => {
                let acting_peer_rsp = ActingResponse::from_parts(rqst_root, trans_id)?;
                Ok(ResponseType::Acting(acting_peer_rsp))
            }
            ExpectedResponse::GetData => {
                unimplemented!();
            }
            ExpectedResponse::PutData => {
                unimplemented!();
            }
            ExpectedResponse::None => Err(DhtError::from_kind(DhtErrorKind::UnsolicitedResponse)),
        }
    }
}
