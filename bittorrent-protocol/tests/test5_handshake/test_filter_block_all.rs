use std::any::Any;
use std::net::SocketAddr;
use std::time::Duration;

use futures::sink::Sink;
use futures::stream::Stream;
use futures::Future;
use tokio::runtime::Runtime;
use tokio::timer::Timeout;

use bittorrent_protocol::handshake::transports::TcpTransport;
use bittorrent_protocol::handshake::{
    DiscoveryInfo, Extensions, FilterDecision, HandshakeFilter, HandshakeFilters,
    HandshakerManagerBuilder, InitiateMessage, Protocol,
};
use bittorrent_protocol::util::bt;
use bittorrent_protocol::util::bt::{InfoHash, PeerId};

use crate::test5_handshake::TimeoutResult;

#[derive(PartialEq, Eq)]
pub struct FilterBlockAll;

impl HandshakeFilter for FilterBlockAll {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn on_addr(&self, _opt_addr: Option<&SocketAddr>) -> FilterDecision {
        FilterDecision::Block
    }
    fn on_prot(&self, _opt_prot: Option<&Protocol>) -> FilterDecision {
        FilterDecision::Block
    }
    fn on_ext(&self, _opt_ext: Option<&Extensions>) -> FilterDecision {
        FilterDecision::Block
    }
    fn on_hash(&self, _opt_hash: Option<&InfoHash>) -> FilterDecision {
        FilterDecision::Block
    }
    fn on_pid(&self, _opt_pid: Option<&PeerId>) -> FilterDecision {
        FilterDecision::Block
    }
}

#[test]
fn test_filter_all() {
    let mut runtime = Runtime::new().unwrap();

    let mut handshaker_one_addr = "127.0.0.1:0".parse().unwrap();
    let handshaker_one_pid = [4u8; bt::PEER_ID_LEN].into();
    let handshaker_one = HandshakerManagerBuilder::new()
        .with_bind_addr(handshaker_one_addr)
        .with_peer_id(handshaker_one_pid)
        .build(TcpTransport)
        .unwrap();
    handshaker_one_addr.set_port(handshaker_one.port());
    // Filter all incoming handshake requests
    handshaker_one.add_filter(FilterBlockAll);


    let mut handshaker_two_addr = "127.0.0.1:0".parse().unwrap();
    let handshaker_two_pid = [5u8; bt::PEER_ID_LEN].into();
    let handshaker_two = HandshakerManagerBuilder::new()
        .with_bind_addr(handshaker_two_addr)
        .with_peer_id(handshaker_two_pid)
        .build(TcpTransport)
        .unwrap();
    handshaker_two_addr.set_port(handshaker_two.port());

    let (_, stream_one) = handshaker_one.into_parts();
    let (sink_two, stream_two) = handshaker_two.into_parts();

    let timeout_result = runtime
        .block_on(
            sink_two
                .send(InitiateMessage::new(
                    Protocol::BitTorrent,
                    [55u8; bt::INFO_HASH_LEN].into(),
                    handshaker_one_addr,
                ))
                .map_err(|_| ())
                .and_then(|_| {
                    let timeout = Timeout::new(Duration::from_millis(50), Default::default())
                        .map(|_| TimeoutResult::TimedOut)
                        .map_err(|_| ());

                    let result_one = stream_one
                        .into_future()
                        .map(|_| TimeoutResult::GotResult)
                        .map_err(|_| ());
                    let result_two = stream_two
                        .into_future()
                        .map(|_| TimeoutResult::GotResult)
                        .map_err(|_| ());

                    result_one
                        .select(result_two)
                        .map(|_| TimeoutResult::GotResult)
                        .map_err(|_| ())
                        .select(timeout)
                        .map(|(item, _)| item)
                        .map_err(|_| ())
                }),
        )
        .unwrap();

    assert_eq!(TimeoutResult::TimedOut, timeout_result);
}
