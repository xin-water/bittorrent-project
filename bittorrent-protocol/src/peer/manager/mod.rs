use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

pub mod builder;
use builder::PeerManagerBuilder;

pub mod peer_info;
use peer_info::PeerInfo;

pub mod error;

use crate::peer::messages::PeerWireProtocolMessage;
use std::net::TcpStream;

//mod task;
mod task;

mod try_clone;
pub use try_clone::TryClone;

// We configure our tick duration based on this, could let users configure this in the future...
const DEFAULT_TIMER_SLOTS: usize = 2048;

/// Manages a set of peers with heartbeating heartbeating.
pub struct PeerManager {
    sink: PeerManagerSink,
    stream: PeerManagerStream,
}

impl PeerManager {
    /// Create a new `PeerManager` from the given `PeerManagerBuilder`.
    pub fn from_builder(builder: PeerManagerBuilder) -> PeerManager {
        let (res_send, res_recv) = mpsc::channel();
        let peers = Arc::new(Mutex::new(HashMap::new()));

        let sink = PeerManagerSink::new(builder, res_send, peers.clone());
        let stream = PeerManagerStream::new(res_recv, peers);

        PeerManager {
            sink: sink,
            stream: stream,
        }
    }

    /// Break the `PeerManager` into a sink and stream.
    ///
    /// The returned sink implements `Clone`.
    pub fn into_parts(self) -> (PeerManagerSink, PeerManagerStream) {
        (self.sink, self.stream)
    }
}

impl PeerManager {

    pub fn send(&mut self, item: IPeerManagerMessage){
        self.sink.send(item)
    }

}

impl PeerManager {

    pub fn poll(&mut self) -> Option<OPeerManagerMessage>{
        self.stream.poll()
    }
}

//----------------------------------------------------------------------------//

/// Sink half of a `PeerManager`.
pub struct PeerManagerSink {
    build: PeerManagerBuilder,
    send: Sender<OPeerManagerMessage>,
    peers: Arc<Mutex<HashMap<PeerInfo, Sender<IPeerManagerMessage>>>>,
}

impl Clone for PeerManagerSink {
    fn clone(&self) -> PeerManagerSink {
        PeerManagerSink {
            build: self.build,
            send: self.send.clone(),
            peers: self.peers.clone(),
        }
    }
}

impl PeerManagerSink {
    fn new(
        build: PeerManagerBuilder,
        send: Sender<OPeerManagerMessage>,
        peers: Arc<Mutex<HashMap<PeerInfo, Sender<IPeerManagerMessage>>>>,
    ) -> PeerManagerSink {
        PeerManagerSink {
            build: build,
            send: send,
            peers: peers,
        }
    }

    fn run_with_lock_sink<F, I>(&mut self, item: I, call: F)
    where
        F: FnOnce(
            I,
            &mut PeerManagerBuilder,
            &mut Sender<OPeerManagerMessage>,
            &mut HashMap<PeerInfo, Sender<IPeerManagerMessage>>,
        ),
    {
        while let Ok(mut guard) = self.peers.try_lock() {
            let _result = call(item, &mut self.build, &mut self.send, &mut *guard);
            break;
        }
    }
}

impl PeerManagerSink {
    pub fn send(&mut self, item: IPeerManagerMessage) {
        match item {
            IPeerManagerMessage::AddPeer(info, peer) => {
                self.run_with_lock_sink((info, peer), |(info, peer), builder, send, peers| {
                    if peers.len() >= builder.peer_capacity() {
                        panic!("bittorrent-protocol_peer: PeerManager Failed To Send AddPeer");
                    } else {
                        match peers.entry(info) {
                            Entry::Occupied(_) => panic!(
                                "bittorrent-protocol_peer: PeerManager Failed To Send AddPeer"
                            ),
                            Entry::Vacant(vac) => {
                                vac.insert(task::run_peer(peer, info, send.clone()));
                            }
                        }
                    }
                })
            }
            IPeerManagerMessage::RemovePeer(info) => {
                self.run_with_lock_sink(info, |info, _, _, peers| {
                    peers
                        .get_mut(&info)
                        .unwrap()
                        .send(IPeerManagerMessage::RemovePeer(info))
                        .expect("bittorrent-protocol_peer: PeerManager Failed To Send RemovePeer");
                })
            }
            IPeerManagerMessage::SendMessage(info, mid, peer_message) => self.run_with_lock_sink(
                (info, mid, peer_message),
                |(info, mid, peer_message), _, _, peers| {
                    peers
                        .get_mut(&info)
                        .unwrap()
                        .send(IPeerManagerMessage::SendMessage(info, mid, peer_message))
                        .expect("bittorrent-protocol_peer: PeerManager Failed to Send SendMessage");
                },
            ),
        }
    }
}

//----------------------------------------------------------------------------//

/// Stream half of a `PeerManager`.
pub struct PeerManagerStream {
    recv: Receiver<OPeerManagerMessage>,
    peers: Arc<Mutex<HashMap<PeerInfo, Sender<IPeerManagerMessage>>>>,
    opt_pending: Option<OPeerManagerMessage>,
}

impl PeerManagerStream {
    fn new(
        recv: Receiver<OPeerManagerMessage>,
        peers: Arc<Mutex<HashMap<PeerInfo, Sender<IPeerManagerMessage>>>>,
    ) -> PeerManagerStream {
        PeerManagerStream {
            recv: recv,
            peers: peers,
            opt_pending: None,
        }
    }

    fn run_with_lock_poll<F, I, G>(
        &mut self,
        item: I,
        call: F,
        not: G,
    ) -> Option<OPeerManagerMessage>
    where
        F: FnOnce(
            I,
            &mut HashMap<PeerInfo, Sender<IPeerManagerMessage>>,
        ) -> Option<OPeerManagerMessage>,
        G: FnOnce(I) -> Option<OPeerManagerMessage>,
    {
        if let Ok(mut guard) = self.peers.try_lock() {
            let result = call(item, &mut *guard);

            // Nothing calling us will return NotReady, so we dont have to push to queue here
            result
        } else {
            // If we couldnt get the lock, stash the item
            self.opt_pending = not(item);

            None
        }
    }
}

impl PeerManagerStream {
    pub fn poll(&mut self) -> Option<OPeerManagerMessage> {
        // Intercept and propogate any messages indicating the peer shutdown so we can remove them from our peer map
        let next_message = self
            .opt_pending
            .take()
            .map(|pending| pending)
            .unwrap_or_else(|| self.recv.recv().unwrap());

        match next_message{
                OPeerManagerMessage::PeerRemoved(info) => self.run_with_lock_poll(
                    info,
                    |info, peers| {
                        peers
                            .remove(&info)
                            .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerRemoved Message With No Matching Peer In Map"));

                       Some(OPeerManagerMessage::PeerRemoved(info))
                    },
                    |info| Some(OPeerManagerMessage::PeerRemoved(info)),
                ),
                OPeerManagerMessage::PeerDisconnect(info) => self.run_with_lock_poll(
                    info,
                    |info, peers| {
                        peers
                            .remove(&info)
                            .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerDisconnect Message With No Matching Peer In Map"));

                        Some(OPeerManagerMessage::PeerDisconnect(info))
                    },
                    |info| Some(OPeerManagerMessage::PeerDisconnect(info)),
                ),
                OPeerManagerMessage::PeerError(info, error) => self.run_with_lock_poll(
                    (info, error),
                    |(info, error), peers| {
                        peers
                            .remove(&info)
                            .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerError Message With No Matching Peer In Map"));

                        Some(OPeerManagerMessage::PeerError(info, error))
                    },
                    |(info, error)| Some(OPeerManagerMessage::PeerError(info, error)),
                ),
                other => Some(other),
            }
    }
}

//----------------------------------------------------------------------------//

/// Trait for giving `PeerManager` message information it needs.
///
/// For any `PeerProtocol` (or plain `Codec`), that wants to be managed
/// by `PeerManager`, it must ensure that it's message type implements
/// this trait so that we have the hooks necessary to manage the peer.
pub trait ManagedMessage {
    /// Retrieve a keep alive message variant.
    fn keep_alive() -> Self;

    /// Whether or not this message is a keep alive message.
    fn is_keep_alive(&self) -> bool;
}

//----------------------------------------------------------------------------//

/// Identifier for matching sent messages with received messages.
pub type MessageId = u64;

/// Message that can be sent to the `PeerManager`.
#[derive(Debug)]
pub enum IPeerManagerMessage {
    /// Add a peer to the peer manager.
    AddPeer(PeerInfo, TcpStream),
    /// Remove a peer from the peer manager.
    RemovePeer(PeerInfo),
    /// Send a message to a peer.
    SendMessage(PeerInfo, MessageId, PeerWireProtocolMessage), // TODO: Support querying for statistics
}

/// Message that can be received from the `PeerManager`.
#[derive(Debug)]
pub enum OPeerManagerMessage {
    /// Message indicating a peer has been added to the peer manager.
    PeerAdded(PeerInfo),
    /// Message indicating a peer has been removed from the peer manager.
    PeerRemoved(PeerInfo),
    /// Message indicating a message has been sent to the given peer.
    SentMessage(PeerInfo, MessageId),
    /// Message indicating we have received a message from a peer.
    ReceivedMessage(PeerInfo, PeerWireProtocolMessage),
    /// Message indicating a peer has disconnected from us.
    ///
    /// Same semantics as `PeerRemoved`, but the peer is not returned.
    PeerDisconnect(PeerInfo),
    /// Message indicating a peer errored out.
    ///
    /// Same semantics as `PeerRemoved`, but the peer is not returned.
    PeerError(PeerInfo, io::Error),
}
