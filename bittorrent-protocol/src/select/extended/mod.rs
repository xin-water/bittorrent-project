use std::collections::{HashMap, VecDeque};

use crate::peer::messages::builders::ExtendedMessageBuilder;
use crate::peer::messages::ExtendedMessage;
use crate::peer::PeerInfo;

use crate::select::error::UberError;
use crate::select::ControlMessage;

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

    pub fn process_message<D>(&mut self, message: IExtendedMessage, d_module: &mut D)
    where
        D: ExtendedListener + ?Sized,
    {
        match message {
            IExtendedMessage::Control(ControlMessage::PeerConnected(info)) => {
                let mut builder = self.builder.clone();

                //调用回调接口 生成拓展builder对象
                builder = d_module.extend(&info, builder);

                let ext_message = builder.build();
                let ext_peer_info = ExtendedPeerInfo::new(Some(ext_message.clone()), None);

                //调用回调接口 更新实现类中的拓展信息
                d_module.on_update(&info, &ext_peer_info);

                self.peers.insert(info, ext_peer_info);
                self.out_queue
                    .push_back(OExtendedMessage::SendExtendedMessage(info, ext_message));
            }
            IExtendedMessage::Control(ControlMessage::PeerDisconnected(info)) => {
                self.peers.remove(&info);
            }
            IExtendedMessage::RecievedExtendedMessage(info, ext_message) => {
                let ext_peer_info = self.peers.get_mut(&info).unwrap();
                ext_peer_info.update_theirs(ext_message);

                //调用回调接口 更新实现类中的拓展信息
                d_module.on_update(&info, &ext_peer_info);
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
