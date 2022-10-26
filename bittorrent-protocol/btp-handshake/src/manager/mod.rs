
use std::cmp;
use std::fmt::Debug;
use std::io;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use rand::{self, Rng};
use tokio::io::{AsyncRead, AsyncWrite};

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;
use btp_util::bt::PeerId;
use btp_util::convert;

use discovery::DiscoveryInfo;
use crate::socket::local_addr::LocalAddr;
use crate::socket::transport::Transport;

use out_msg::CompleteMessage;
use crate::message::extensions::Extensions;
use in_msg::InitiateMessage;

use crate::filter::filters::Filters;
use crate::filter::{HandshakeFilter, HandshakeFilters};

use crate::handler;
use crate::handler::srart_handler_task;

pub mod config;
pub mod out_msg;
pub mod in_msg;
pub mod discovery;

use self::config::HandshakerConfig;
const DEFAULT_V4_PORT: u16 = 22222;
const DEFAULT_V4_ADDR: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

/// Build configuration for `Handshaker` object creation.
#[derive(Copy, Clone)]
pub struct HandshakerManagerBuilder {
    bind: SocketAddr,
    port: u16,
    pid: PeerId,
    ext: Extensions,
    config: HandshakerConfig,
}

impl HandshakerManagerBuilder {
    /// Create a new `HandshakerBuilder`.
    pub fn new() -> HandshakerManagerBuilder {

        let default_sock_addr = SocketAddr::V4(SocketAddrV4::new(DEFAULT_V4_ADDR, DEFAULT_V4_PORT));

        let seed = rand::thread_rng().next_u32();
        let default_peer_id = PeerId::from_bytes(&convert::four_bytes_to_array(seed));

        HandshakerManagerBuilder {
            bind: default_sock_addr,
            port: DEFAULT_V4_PORT,
            pid: default_peer_id,
            ext: Extensions::new(),
            config: HandshakerConfig::default(),
        }
    }

    /// Address that the host will listen on.
    ///
    /// Defaults to IN_ADDR_ANY using port 0 (any free port).
    pub fn with_bind_addr(&mut self, addr: SocketAddr) -> &mut HandshakerManagerBuilder {
        self.bind = addr;

        self
    }

    pub fn with_bind_port(&mut self, bind_port: u16) -> &mut HandshakerManagerBuilder {
        self.bind = SocketAddr::V4(SocketAddrV4::new(DEFAULT_V4_ADDR, bind_port));

        self
    }

    /// Port that external peers should connect on.
    ///
    /// Defaults to the port that is being listened on (will only work if the
    /// host is not natted).
    pub fn with_open_port(&mut self, port: u16) -> &mut HandshakerManagerBuilder {
        self.port = port;

        self
    }

    /// Peer id that will be advertised when handshaking with other peers.
    ///
    /// Defaults to a random SHA-1 hash; official clients should use an encoding scheme.
    ///
    /// See http://www.bittorrent.org/beps/bep_0020.html.
    pub fn with_peer_id(&mut self, peer_id: PeerId) -> &mut HandshakerManagerBuilder {
        self.pid = peer_id;

        self
    }

    /// Extensions supported by our client, advertised to the peer when handshaking.
    pub fn with_extensions(&mut self, ext: Extensions) -> &mut HandshakerManagerBuilder {
        self.ext = ext;

        self
    }

    /// Configuration that will be used to alter the internal behavior of handshaking.
    ///
    /// This will typically not need to be set unless you know what you are doing.
    pub fn with_config(&mut self, config: HandshakerConfig) -> &mut HandshakerManagerBuilder {
        self.config = config;

        self
    }

    /// Build a `Handshaker` over the given `Transport` with a `Remote` instance.
    pub async fn build<T>(&self, transport: T) -> io::Result<HandshakerManager<T::Socket>>
        where
            T: Transport + 'static + Send + Sync + Debug,
            <T as Transport>::Socket: Send,
    {
        HandshakerManager::with_builder(self, transport).await
    }
}

//----------------------------------------------------------------------------------//

/// Handshaker which is both `Stream` and `Sink`.
pub struct HandshakerManager<S> {
    sink: HandshakerManagerSink,
    stream: HandshakerManagerStream<S>,
}

impl<S> HandshakerManager<S> {
    /// Splits the `Handshaker` into its parts.
    ///
    /// This is an enhanced version of `Stream::split` in that the returned `Sink` implements
    /// `DiscoveryInfo` so it can be cloned and passed in to different peer discovery services.
    pub fn into_parts(self) -> (HandshakerManagerSink, HandshakerManagerStream<S>) {
        (self.sink, self.stream)
    }
}

impl<S> DiscoveryInfo for HandshakerManager<S> {
    fn port(&self) -> u16 {
        self.sink.port()
    }

    fn peer_id(&self) -> PeerId {
        self.sink.peer_id()
    }
}

impl<S> HandshakerManager<S>
    where
        S: AsyncRead + AsyncWrite + 'static + Send + Unpin + Debug ,
{
  async fn with_builder<T>(
        builder: &HandshakerManagerBuilder,
        transport: T,
    ) -> io::Result<HandshakerManager<T::Socket>>
        where
            T: Transport<Socket = S> + 'static + Send + Sync,
    {
        let listener = transport.listen(&builder.bind).await?;

        // Resolve our "real" public port
        let open_port = if builder.port == 0 {
            listener.local_addr()?.port()
        } else {
            builder.port
        };

        let config = builder.config;

        let (addr_send, addr_recv) = mpsc::channel(config.sink_buffer_size());
        let (sock_send, sock_recv) = mpsc::unbounded_channel();

        let filters = Filters::new();

        srart_handler_task(builder.pid,builder.ext,transport,listener,addr_recv,sock_send,filters.clone()).await;

        let sink = HandshakerManagerSink::new(addr_send, open_port, builder.pid, filters);
        let stream = HandshakerManagerStream::new(sock_recv);

        Ok(HandshakerManager {
            sink: sink,
            stream: stream,
        })
    }
}


impl<S> HandshakerManager<S> {

   pub async fn send(
        &mut self,
        item: InitiateMessage,
    ) ->  Result<(), SendError<InitiateMessage>> {
        self.sink.send(item).await
    }

}

impl<S> HandshakerManager<S> {

   pub async fn poll(&mut self) -> Option<CompleteMessage<S>> {
        self.stream.poll().await
    }
}

impl<S> HandshakeFilters for HandshakerManager<S> {
    fn add_filter<F>(&self, filter: F)
        where
            F: HandshakeFilter + PartialEq + Eq + Send + Sync + 'static,
    {
        self.sink.add_filter(filter);
    }

    fn remove_filter<F>(&self, filter: F)
        where
            F: HandshakeFilter + PartialEq + Eq + Send + Sync + 'static,
    {
        self.sink.remove_filter(filter);
    }

    fn clear_filters(&self) {
        self.sink.clear_filters();
    }
}

//----------------------------------------------------------------------------------//

/// `Sink` portion of the `Handshaker` for initiating handshakes.
#[derive(Clone)]
pub struct HandshakerManagerSink {
    send: mpsc::Sender<InitiateMessage>,
    port: u16,
    pid: PeerId,
    filters: Filters,
}

impl HandshakerManagerSink {
    fn new(
        send: mpsc::Sender<InitiateMessage>,
        port: u16,
        pid: PeerId,
        filters: Filters,
    ) -> HandshakerManagerSink {
        HandshakerManagerSink {
            send: send,
            port: port,
            pid: pid,
            filters: filters,
        }
    }
}

impl DiscoveryInfo for HandshakerManagerSink {
    fn port(&self) -> u16 {
        self.port
    }

    fn peer_id(&self) -> PeerId {
        self.pid
    }
}

impl  HandshakerManagerSink {

   pub async fn send(
        &mut self,
        item: InitiateMessage,
    ) -> Result<(), SendError<InitiateMessage>> {

        self.send.send(item).await
    }

}

impl HandshakeFilters for HandshakerManagerSink {
    fn add_filter<F>(&self, filter: F)
        where
            F: HandshakeFilter + PartialEq + Eq + Send + Sync + 'static,
    {
        self.filters.add_filter(filter);
    }

    fn remove_filter<F>(&self, filter: F)
        where
            F: HandshakeFilter + PartialEq + Eq + Send + Sync + 'static,
    {
        self.filters.remove_filter(filter);
    }

    fn clear_filters(&self) {
        self.filters.clear_filters();
    }
}

//----------------------------------------------------------------------------------//

/// `Stream` portion of the `Handshaker` for completed handshakes.
pub struct HandshakerManagerStream<S> {
    recv: mpsc::UnboundedReceiver<CompleteMessage<S>>,
}

impl<S> HandshakerManagerStream<S> {
    fn new(recv: mpsc::UnboundedReceiver<CompleteMessage<S>>) -> HandshakerManagerStream<S> {
        HandshakerManagerStream { recv: recv }
    }
}

impl<S>  HandshakerManagerStream<S> {

   pub async fn poll(&mut self) -> Option<CompleteMessage<S>> {
        self.recv.recv().await
    }
}

