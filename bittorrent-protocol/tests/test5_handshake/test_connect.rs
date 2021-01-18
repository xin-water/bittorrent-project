use bittorrent_protocol::handshake::transports::TcpTransport;
use bittorrent_protocol::handshake::{DiscoveryInfo, HandshakerManagerBuilder, InitiateMessage, Protocol};
use bittorrent_protocol::util::bt;

#[test]
fn positive_connect() {

    let mut handshaker_one_addr = "127.0.0.1:0".parse().unwrap();
    let handshaker_one_pid = [4u8; bt::PEER_ID_LEN].into();
    let mut handshaker_one = HandshakerManagerBuilder::new()
        .with_bind_addr(handshaker_one_addr)
        .with_peer_id(handshaker_one_pid)
        .build(TcpTransport)
        .unwrap();
    handshaker_one_addr.set_port(handshaker_one.port());


    let mut handshaker_two_addr = "127.0.0.1:0".parse().unwrap();
    let handshaker_two_pid = [5u8; bt::PEER_ID_LEN].into();
    let mut handshaker_two = HandshakerManagerBuilder::new()
        .with_bind_addr(handshaker_two_addr)
        .with_peer_id(handshaker_two_pid)
        .build(TcpTransport)
        .unwrap();
    handshaker_two_addr.set_port(handshaker_two.port());


    handshaker_one.send(InitiateMessage::new(Protocol::BitTorrent, [55u8; bt::INFO_HASH_LEN].into(), handshaker_two_addr ));
    let (Protocol, Extensions, InfoHash, PeerId, SocketAddr, S) = handshaker_two.poll().unwrap().into_parts();


    assert_eq!(handshaker_one_pid, PeerId);

    // Result from handshaker one should match handshaker two's listen address
    //assert_eq!(handshaker_two_addr, S.peer_addr().unwrap());
}
