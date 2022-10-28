use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io;
use std::io::{Read, Write};
use tokio::sync::mpsc::{self, Receiver, Sender, UnboundedReceiver};
use std::sync::{Arc, Mutex};

pub mod builder;
use builder::PeerManagerBuilder;

use crate::handler::peer_info::PeerInfo;

pub mod error;

use crate::messages::PeerWireProtocolMessage;
use std::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::error::SendError;

use crate::split::Split;
use crate::handler::start_peer_task;

// We configure our tick duration based on this, could let users configure this in the future...
const DEFAULT_TIMER_SLOTS: usize = 2048;

/// Manages a set of peers with heartbeating heartbeating.
pub struct PeerManager<S> {
    sink: PeerManagerSink<S>,
    stream: PeerManagerStream,
}

impl<S> PeerManager<S>
where  S: AsyncWrite + AsyncRead + Send + 'static + Split + Debug

{
    /// Create a new `PeerManager` from the given `PeerManagerBuilder`.
    pub fn from_builder(builder: PeerManagerBuilder) -> PeerManager<S> {


        let (command_tx,msg_rx) = start_peer_task(builder);

        let sink = PeerManagerSink::new(command_tx);
        let stream = PeerManagerStream::new(msg_rx);


        PeerManager {
            sink: sink,
            stream: stream,
        }
    }

    /// Break the `PeerManager` into a sink and stream.
    ///
    /// The returned sink implements `Clone`.
    pub fn into_parts(self) -> (PeerManagerSink<S>, PeerManagerStream) {
        (self.sink, self.stream)
    }
}

impl<S> PeerManager<S>
    where S: AsyncRead + AsyncWrite + Send + 'static,
    {

    pub async fn send(&mut self, item: IPeerManagerMessage<S>) -> Result<(), SendError<IPeerManagerMessage<S>>> {
        self.sink.send(item).await
    }

}

impl<S> PeerManager<S> {

    pub async fn poll(&mut self) -> Option<OPeerManagerMessage>{
        self.stream.rx().await
    }
}

//----------------------------------------------------------------------------//

/// Sink half of a `PeerManager`.
pub struct PeerManagerSink<S> {
    send: Sender<IPeerManagerMessage<S>>,
}

impl<S> Clone for PeerManagerSink<S> {
    fn clone(&self) -> PeerManagerSink<S> {
        PeerManagerSink {
            send: self.send.clone(),
        }
    }
}

impl<S> PeerManagerSink<S> {
    fn new(
        send: Sender<IPeerManagerMessage<S>>,
    ) -> PeerManagerSink<S> {
        PeerManagerSink {
            send: send,
        }
    }
}

impl<S> PeerManagerSink<S>
where S: AsyncRead + AsyncWrite + Send + 'static,
{

    pub async fn send(&mut self, item: IPeerManagerMessage<S>) -> Result<(), SendError<IPeerManagerMessage<S>>> {
        self.send.send(item).await
    }
}

//----------------------------------------------------------------------------//

/// Stream half of a `PeerManager`.
pub struct PeerManagerStream{
    recv: UnboundedReceiver<OPeerManagerMessage>,
}

impl PeerManagerStream {
    fn new(
        recv: UnboundedReceiver<OPeerManagerMessage>,
    ) -> PeerManagerStream {
        PeerManagerStream {
            recv: recv,
        }
    }
}

impl PeerManagerStream {
    pub async fn rx(&mut self) -> Option<OPeerManagerMessage> {
        self.recv.recv().await
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
pub enum IPeerManagerMessage<S>{

    /// Add a peer to the peer manager.
    AddPeer(PeerInfo, S),
    /// Remove a peer from the peer manager.
    RemovePeer(PeerInfo),
    /// Send a message to a peer.
    SendMessage(PeerInfo, MessageId, PeerWireProtocolMessage), // TODO: Support querying for statistics
    Shutdown
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
