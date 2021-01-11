use bittorrent_protocol::handshake::{BTHandshaker, BTPeer, Handshaker};
use bittorrent_protocol::peer::{
    IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder,
};
use bittorrent_protocol::util::sha::ShaHash;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

fn main() {
   // "-qB430A-fMd.S1AYOOVa"    "-qB430A-ltWN!p4KL6C2"
    let ( send1, recv1): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();
    let mut handshaker1 = BTHandshaker::new(
        send1,
        "127.0.0.1:0".parse().unwrap(),
        (*b"-UT2060-111111111111").into(),
    )
    .unwrap();

    let (send2, recv2): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();
    let handshaker2 = BTHandshaker::new(
        send2,
        "127.0.0.1:0".parse().unwrap(),
        (*b"-UT2060-222222222222").into(),
    )
    .unwrap();

    let bt_hash_hex = "3c4fffbc472671e16a6cf7b6473ba9a8bf1b1a3a";
    let bt_hash = hex::decode(bt_hash_hex).unwrap();
    let hash = ShaHash::from_hash(bt_hash.as_slice()).unwrap();
    println!("{:?}",&bt_hash);
    handshaker1.register_hash(hash);
    handshaker2.register_hash(hash);
    let addr2 = format!("127.0.0.1:{:?}", handshaker2.port());

    handshaker1.connect(
        None,
        hash,
        addr2.parse().unwrap(),
    );

    let peer_one = recv1.recv().unwrap().destory();
    let peer_two = recv2.recv().unwrap().destory();
    println!("\
    peer_id_1: {:?}\n\
    peer_id_1: {:?}\n\
    peer_id_2: {:?}\n\
    peer_id_2: {:?}",
    peer_one.2.as_ref(),
    String::from_utf8_lossy( peer_one.2.as_ref()),
    peer_two.2.as_ref(),
    String::from_utf8_lossy( peer_two.2.as_ref()),
    );

    let (mut manager_sink, mut manager_steam) = PeerManagerBuilder::new()
        .with_peer_capacity(1)
        .build()
        .into_parts();

    // Create two peers
    let peer_one_info = PeerInfo::new(peer_one.0.local_addr().unwrap(), peer_one.2, peer_one.1);
    let peer_two_info = PeerInfo::new(peer_two.0.local_addr().unwrap(), peer_two.2, peer_two.1);

    // Add peer one to the manager
    manager_sink.send(IPeerManagerMessage::AddPeer(peer_one_info, peer_one.0));

    // Check that peer one was added
    let response = manager_steam.poll().unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            println!("\nPeerAdded\n\
            adr: {:?}\n\
            add: {:?}\n\
            spn: {:?}\n",
            peer_one_info.peer_id().as_ref(),
            String::from_utf8_lossy( peer_one_info.peer_id().as_ref()),
            String::from_utf8_lossy( info.peer_id().as_ref())
            );
        }
        _ => panic!("Unexpected First Peer Manager Response"),
    };

    // Remove peer one from the manager
    manager_sink.send(IPeerManagerMessage::RemovePeer(peer_one_info));

    // Check that peer one was removed
    let response = manager_steam.poll().unwrap();

    match response {
        OPeerManagerMessage::PeerRemoved(info) => {
            println!("PeerRemoved\n\
            adr: {:?}\n\
            rem: {:?}\n\
            spn: {:?}\n",
            peer_one_info.peer_id().as_ref(),
            String::from_utf8_lossy( peer_one_info.peer_id().as_ref()),
            String::from_utf8_lossy( info.peer_id().as_ref())
            )
        }

        _ => panic!("Unexpected Third Peer Manager Response"),
    };

    // Try to add peer two, but make sure it goes through
    manager_sink.send(IPeerManagerMessage::AddPeer(peer_two_info, peer_two.0));

    let response = manager_steam.poll().unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            println!("PeerAdded\n\
            adr: {:?}\n\
            add: {:?}\n\
            spn: {:?}\n",
            peer_two_info.peer_id().as_ref(),
            String::from_utf8_lossy( peer_two_info.peer_id().as_ref()),
            String::from_utf8_lossy( info.peer_id().as_ref())
            )
        }

        _ => panic!("Unexpected Fourth Peer Manager Response"),
    };

   std::thread::sleep(Duration::from_secs(3));
}
