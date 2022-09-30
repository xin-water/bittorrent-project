use std::collections::{HashMap, VecDeque};

use btp_peer::messages::builders::ExtendedMessageBuilder;
use btp_peer::messages::ExtendedMessage;
use btp_peer::PeerInfo;

use crate::error::UberError;
use crate::ControlMessage;
use crate::uber::DiscoveryTrait;

/// Enumeration of extended messages that can be sent to the extended module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IExtendedMessage {
    Control(ControlMessage),
    RecievedExtendedMessage(PeerInfo, ExtendedMessage),
}

/// Enumeration of extended messages that can be received from the extended module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OExtendedMessage {
    SendExtendedMessage(PeerInfo, ExtendedMessage),
}

/// Container for both the local and remote `ExtendedMessage`.
pub struct ExtendedPeerInfo {
    ours: Option<ExtendedMessage>,
    theirs: Option<ExtendedMessage>,
}

impl ExtendedPeerInfo {
    pub fn new(ours: Option<ExtendedMessage>, theirs: Option<ExtendedMessage>) -> ExtendedPeerInfo {
        ExtendedPeerInfo {
            ours: ours,
            theirs: theirs,
        }
    }

    pub fn update_ours(&mut self, message: ExtendedMessage) {
        self.ours = Some(message);
    }

    pub fn update_theirs(&mut self, message: ExtendedMessage) {
        self.theirs = Some(message);
    }

    pub fn our_message(&self) -> Option<&ExtendedMessage> {
        self.ours.as_ref()
    }

    pub fn their_message(&self) -> Option<&ExtendedMessage> {
        self.theirs.as_ref()
    }
}

//------------------------------------------------------------------------------//

pub struct ExtendedModule {
    builder: ExtendedMessageBuilder,
    peers: HashMap<PeerInfo, ExtendedPeerInfo>,
    out_queue: VecDeque<OExtendedMessage>,
}

/// Trait for a module to take part in constructing the extended message for a peer.
/// 接收到拓展消息时的 回调处理接口
pub trait ExtendedListener {
    /// Extend the given extended message builder for the given peer.
    fn extend(&self, _info: &PeerInfo, _builder: ExtendedMessageBuilder) -> ExtendedMessageBuilder {
        _builder
    }

    /// One or both sides of a peer connection had their extended information updated.
    ///
    /// This can be called multiple times for any given peer as extension information updates.
    fn on_update(&mut self, _info: &PeerInfo, _extended: &ExtendedPeerInfo) {}
}

impl ExtendedModule {
    pub fn new(builder: ExtendedMessageBuilder) -> ExtendedModule {
        ExtendedModule {
            builder: builder,
            peers: HashMap::new(),
            out_queue: VecDeque::new(),
        }
    }

    pub fn process_message(&mut self, message: IExtendedMessage, d_modules: &mut [Box<(dyn DiscoveryTrait)>])
    {
        match message {
            IExtendedMessage::Control(ControlMessage::PeerConnected(info)) => {
                let mut builder = self.builder.clone();

                //调用回调接口 生成拓展builder对象
                for d_module in d_modules.iter() {
                    let temp_builder = builder;
                    builder = d_module.extend(&info, temp_builder);
                }

                let ext_message = builder.build();
                let ext_peer_info = ExtendedPeerInfo::new(Some(ext_message.clone()), None);

                //调用回调接口 更新实现类中的拓展信息
                for d_module in d_modules {
                    d_module.on_update(&info, &ext_peer_info);
                }

                self.peers.insert(info, ext_peer_info);
                self.out_queue
                    .push_back(OExtendedMessage::SendExtendedMessage(info, ext_message));
            }
            IExtendedMessage::Control(ControlMessage::PeerDisconnected(info)) => {
                self.peers.remove(&info);
            }
            IExtendedMessage::RecievedExtendedMessage(info, ext_message) => {
                let opt_ext_peer_info =self.peers.get_mut(&info);
                if opt_ext_peer_info.is_some(){

                    let ext_peer_info=opt_ext_peer_info.unwrap();

                    ext_peer_info.update_theirs(ext_message);
                    //调用回调接口 更新实现类中的拓展信息
                    for d_module in d_modules {
                        d_module.on_update(&info, &ext_peer_info);
                    }
                }
            }
            _ => (),
        }
    }
}

impl ExtendedModule {
    pub(crate) fn poll(&mut self) -> Result<Option<OExtendedMessage>, UberError> {
        let opt_message = self.out_queue.pop_front();
        Ok(opt_message)
    }
}
