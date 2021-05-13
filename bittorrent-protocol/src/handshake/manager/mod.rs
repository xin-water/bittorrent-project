
use std::cmp;
use std::io;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;
use rand::{self, Rng};

use crossbeam::channel::{bounded, Receiver, SendError, Sender};
use crate::util::bt::PeerId;
use crate::util::convert;

use crate::handshake::discovery::DiscoveryInfo;
use crate::handshake::local_addr::LocalAddr;
use crate::handshake::transport::Transport;

use crate::handshake::message::complete::CompleteMessage;
use crate::handshake::message::extensions::Extensions;
use crate::handshake::message::initiate::InitiateMessage;

use crate::handshake::filter::filters::Filters;
use crate::handshake::filter::{HandshakeFilter, HandshakeFilters};

use crate::handshake::handler;
use crate::handshake::handler::listener::ListenerHandler;
use crate::handshake::handler::timer::HandshakeTimer;
use crate::handshake::handler::{handshaker, initiator, HandshakeType};

pub mod config;
use self::config::HandshakerConfig;

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
        let default_v4_addr = Ipv4Addr::new(0, 0, 0, 0);
        let default_v4_port = 22222;

        let default_sock_addr = SocketAddr::V4(SocketAddrV4::new(default_v4_addr, default_v4_port));

        let seed = rand::thread_rng().next_u32();
        let default_peer_id = PeerId::from_bytes(&convert::four_bytes_to_array(seed));

        HandshakerManagerBuilder {
            bind: default_sock_addr,
            port: default_v4_port,
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
    pub fn build<T>(&self, transport: T) -> io::Result<HandshakerManager<T::Socket>>
        where
            T: Transport + 'static + Send ,
            <T as Transport>::Socket: Send,
    {
        HandshakerManager::with_builder(self, transport)
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
        S: Read + Write + 'static + Send ,
{
    fn with_builder<T>(
        builder: &HandshakerManagerBuilder,
        transport: T,
    ) -> io::Result<HandshakerManager<T::Socket>>
        where
            T: Transport<Socket = S> + 'static + Send,
    {
        let listener = transport.listen(&builder.bind)?;

        // Resolve our "real" public port
        let open_port = if builder.port == 0 {
            listener.local_addr()?.port()
        } else {
            builder.port
        };

        let config = builder.config;
        let (addr_send, addr_recv) = bounded(config.sink_buffer_size());
        let (hand_send, hand_recv) = bounded(config.wait_buffer_size());
        let (sock_send, sock_recv) = bounded(config.done_buffer_size());

        let filters = Filters::new();
        let (handshake_timer, initiate_timer) =
            configured_handshake_timers(config.handshake_timeout(), config.connect_timeout());

        // Hook up our pipeline of handlers which will take some connection info, process it, and forward it
        handler::loop_handler(
            addr_recv,
            (transport, filters.clone(), initiate_timer),
            initiator::initiator_handler,
            hand_send.clone(),
        );
        handler::loop_handler(
            listener,
            filters.clone(),
            |item, context| { ListenerHandler::new(item, context).poll() },
            hand_send,
        );
        handler::loop_handler(
            hand_recv,
            (builder.ext, builder.pid, filters.clone(), handshake_timer),
            handshaker::execute_handshake,
            sock_send,
        );

        let sink = HandshakerManagerSink::new(addr_send, open_port, builder.pid, filters);
        let stream = HandshakerManagerStream::new(sock_recv);

        Ok(HandshakerManager {
            sink: sink,
            stream: stream,
        })
    }
}

/// Configure a timer wheel and create a `HandshakeTimer`.
fn configured_handshake_timers(
    duration_one: Duration,
    duration_two: Duration,
) -> (HandshakeTimer, HandshakeTimer) {
    (
        HandshakeTimer::new( duration_one),
        HandshakeTimer::new(duration_two),
    )
}

impl<S> HandshakerManager<S> {

   pub fn send(
        &mut self,
        item: InitiateMessage,
    ) ->  Result<(), SendError<InitiateMessage>> {
        self.sink.send(item)
    }

}

impl<S> HandshakerManager<S> {

   pub fn poll(&mut self) -> Result<CompleteMessage<S>, ()> {
        self.stream.poll()
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
    send: Sender<InitiateMessage>,
    port: u16,
    pid: PeerId,
    filters: Filters,
}

impl HandshakerManagerSink {
    fn new(
        send: Sender<InitiateMessage>,
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

   pub fn send(
        &mut self,
        item: InitiateMessage,
    ) -> Result<(), SendError<InitiateMessage>> {

        self.send.send(item)
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
    recv: Receiver<CompleteMessage<S>>,
}

impl<S> HandshakerManagerStream<S> {
    fn new(recv: Receiver<CompleteMessage<S>>) -> HandshakerManagerStream<S> {
        HandshakerManagerStream { recv: recv }
    }
}

impl<S>  HandshakerManagerStream<S> {

   pub fn poll(&mut self) -> Result<CompleteMessage<S>, ()> {
        self.recv.recv().map_err(|_|())
    }
}

