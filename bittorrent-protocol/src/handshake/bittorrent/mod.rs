use std::collections::HashSet;
use std::io::{self};
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use super::Handshaker;
use crate::util::bt::{InfoHash, PeerId};

mod handler;

mod base;
pub use crate::handshake::bittorrent::base::BTPeer;
use crate::handshake::bittorrent::base::Task;
use std::sync::mpsc::Sender;

const MAX_PROTOCOL_LEN: usize = 255;
const BTP_10_PROTOCOL: &'static str = "BitTorrent protocol";

pub struct InnerBTHandshaker<T>
where
    T: Send,
{
    // Using a priority channel because shutdown messages cannot
    // afford to be lost. Generally the handler will set the capacity
    // on the channel to 1 less than the real capacity which gives us
    // room for a shutdown message in the absolute worst case.
    chan: Sender<Task<T>>,
    interest: Arc<RwLock<HashSet<InfoHash>>>,
    port: u16,
    pid: PeerId,
}

impl<T> Drop for InnerBTHandshaker<T>
where
    T: Send,
{
    fn drop(&mut self) {
        self.chan.send(Task::Shutdown).unwrap();
    }
}

/// Bittorrent TCP peer handshaker.
pub struct BTHandshaker<T>
where
    T: Send,
{
    inner: Arc<InnerBTHandshaker<T>>,
}

impl<T> Clone for BTHandshaker<T>
where
    T: Send,
{
    fn clone(&self) -> BTHandshaker<T> {
        BTHandshaker {
            inner: self.inner.clone(),
        }
    }
}

impl<T> BTHandshaker<T>
where
    T: From<BTPeer> + Send + 'static,
{
    /// Create a new BTHandshaker with the given PeerId and bind address which will
    /// forward metadata and handshaken connections onto the provided channel.
    pub fn new(chan: Sender<T>, listen: SocketAddr, pid: PeerId) -> io::Result<BTHandshaker<T>>
    {
        BTHandshaker::with_protocol(chan, listen, pid, BTP_10_PROTOCOL)
    }

    /// Similar to BTHandshaker::new() but allows a client to specify a custom protocol
    /// that the handshaker will specify during the handshake.
    ///
    /// Panics if the length of the provided protocol exceeds 255 bytes.
    pub fn with_protocol(
        chan: Sender<T>,
        listen: SocketAddr,
        pid: PeerId,
        protocol: &'static str,
    ) -> io::Result<BTHandshaker<T>> {

        if protocol.len() > MAX_PROTOCOL_LEN {
            panic!(
                "bittorrent-protocol_handshake: BTHandshaker Protocol Length Cannot Exceed {}",
                MAX_PROTOCOL_LEN
            );
        }
        let interest = Arc::new(RwLock::new(HashSet::new()));
        let (chan, port) =
            handler::spawn_handshaker(chan, listen, pid, protocol, interest.clone())?;

        Ok(BTHandshaker {
            inner: Arc::new(InnerBTHandshaker {
                chan: chan,
                interest: interest,
                port: port,
                pid: pid,
            }),
        })
    }

    /// Register interest for the given InfoHash allowing connections for the given InfoHash to succeed. Connections
    /// already in the handshaking process may not be effected by this call.
    ///
    /// By default, a BTHandshaker will be interested in zero InfoHashs.
    ///
    /// This is a blocking operation.
    pub fn register_hash(&self, hash: InfoHash) {
        self.inner.interest.write().unwrap().insert(hash);
    }

    /// Deregister interest for the given InfoHash causing connections for the given InfoHash to fail. Connections
    /// already in the handshaking process may not be effected by this call.
    ///
    /// By default, a BTHandshaker will be interested in zero InfoHashs.
    ///
    /// This is a blocking operation.
    pub fn deregister_hash(&self, hash: InfoHash) {
        self.inner.interest.write().unwrap().remove(&hash);
    }

    pub fn drop(&mut self) {
        drop(self.inner.clone());
    }
}

impl<T> Handshaker for BTHandshaker<T>
where
    T: Send,
{
    type MetadataEnvelope = T;

    fn id(&self) -> PeerId {
        self.inner.pid
    }

    fn port(&self) -> u16 {
        self.inner.port
    }

    fn connect(&mut self, expected: Option<PeerId>, hash: InfoHash, addr: SocketAddr) {
        self.inner
            .chan
            .send(Task::Connect(expected, hash, addr)).unwrap();
    }

    fn metadata(&mut self, data: Self::MetadataEnvelope) {
        self.inner.chan.send(Task::Metadata(data)).unwrap();
    }
}
