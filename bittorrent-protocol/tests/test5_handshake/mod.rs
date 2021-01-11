use std::mem::{self};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self};
use std::time::Duration;

use bittorrent_protocol::handshake::{BTHandshaker, BTPeer, Handshaker};
use bittorrent_protocol::util::bt;

#[test]
pub fn my_print() {
    println!("test handshake");
}

#[test]
fn positive_make_conenction() {
    // Assign a listen address and peer id for each handshaker
    let ip = Ipv4Addr::new(127, 0, 0, 1);
    let (addr_one, addr_two) = (
        SocketAddr::V4(SocketAddrV4::new(ip, 0)),
        SocketAddr::V4(SocketAddrV4::new(ip, 0)),
    );
    let (peer_id_one, peer_id_two) = ([1u8; bt::PEER_ID_LEN].into(), [0u8; bt::PEER_ID_LEN].into());

    // Create receiving channels
    let (send_one, recv_one): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();
    let (send_two, recv_two): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();

    // Create two handshakers
    let mut handshaker_one = BTHandshaker::new(send_one, addr_one, peer_id_one).unwrap();
    let handshaker_two = BTHandshaker::new(send_two, addr_two, peer_id_two).unwrap();

    // Register both handshakers for the same info hash
    let info_hash = [0u8; bt::INFO_HASH_LEN].into();
    handshaker_one.register_hash(info_hash);
    handshaker_two.register_hash(info_hash);

    // Allow the handshakers to connect to each other
    thread::sleep(Duration::from_millis(100));

    // Get the address and bind port for handshaker two
    let handshaker_two_addr = SocketAddr::V4(SocketAddrV4::new(ip, handshaker_two.port()));

    // Connect to handshaker two from handshaker one
    handshaker_one.connect(Some(peer_id_two), info_hash, handshaker_two_addr);

    // Allow the handshakers to connect to each other
    thread::sleep(Duration::from_millis(100));

    // Should receive the peer from both handshakers
    match (recv_one.try_recv(), recv_two.try_recv()) {
        (Ok(_), Ok(_)) => (),
        _ => panic!("Failed to find peers on one or both handshakers..."),
    };
}

#[test]
fn positive_shutdown_on_drop() {
    // Create our handshaker addresses and ids
    let ip = Ipv4Addr::new(127, 0, 0, 1);
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, 0));
    let peer_id = [0u8; bt::PEER_ID_LEN].into();

    let (send, recv): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();

    // Create the handshaker
    let handshaker = BTHandshaker::new(send, addr, peer_id).unwrap();

    // Subscribe to a specific info hash
    let info_hash = [1u8; bt::INFO_HASH_LEN].into();
    handshaker.register_hash(info_hash);

    // Clone the handshaker so there are two copies of it
    let handshaker_clone = handshaker.clone();

    // Drop one of the copies of the handshaker
    mem::drop(handshaker_clone);

    // Allow the event loop to process the shutdown that should NOT have been fired
    thread::sleep(Duration::from_millis(100));

    // Make sure that we can still receive values on our channel
    assert_eq!(recv.try_recv().unwrap_err(), TryRecvError::Empty);

    // Drop the other copy of the handshaker so there are no more copies around
    mem::drop(handshaker);

    // Allow the event loop to process the shutdown that should have been fired
    thread::sleep(Duration::from_millis(100));

    // Assert that the channel hunp up (because of the shutdown)
    assert_eq!(recv.try_recv().unwrap_err(), TryRecvError::Disconnected);
}
