use log::{debug, info, LevelFilter};
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Config, Logger, Root},
    encode::pattern::PatternEncoder,
    filter::threshold::ThresholdFilter,
};
use std::io;
use std::net::{SocketAddr, IpAddr, Ipv4Addr, TcpListener, TcpStream};

use bittorrent_protocol::handshake::Extensions;
use bittorrent_protocol::peer::messages::PeerWireProtocolMessage;
use bittorrent_protocol::peer::{
    IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder,
};

use bittorrent_protocol::util::bt;

fn init_log() {
    let stdout = ConsoleAppender::builder()
        .target(Target::Stdout)
        .encoder(Box::new(PatternEncoder::new(
            "[Console] {d} - {l} -{t} - {m}{n}",
        )))
        .build();

    let file = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "[File] {d} - {l} - {t} - {m}{n}",
        )))
        .build("log/log.log")
        .unwrap();

    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .appender(Appender::builder().build("file", Box::new(file)))
        .build(
            Root::builder()
                .appender("stdout")
                .appender("file")
                .build(LevelFilter::Trace),
        )
        .unwrap();

    let _ = log4rs::init_config(config).unwrap();
}

fn main() {

    // Start logger
    init_log();
    info!("start run .......");

    let mut manager = PeerManagerBuilder::new()
        .with_peer_capacity(1)
        .build();

    // Create two peers
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let tcplisten=TcpListener::bind(&socket).unwrap();
    let listen_addr= tcplisten.local_addr().unwrap();
    let  peer_one  = TcpStream::connect(&listen_addr).unwrap();

    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let tcplisten=TcpListener::bind(&socket).unwrap();
    let listen_addr= tcplisten.local_addr().unwrap();
    let peer_two  = TcpStream::connect(&listen_addr).unwrap();

    let peer_one_info = PeerInfo::new(
        peer_one.peer_addr().unwrap(),
        [0u8; bt::PEER_ID_LEN].into(),
        [0u8; bt::INFO_HASH_LEN].into(),
        Extensions::new(),
    );

    let peer_two_info = PeerInfo::new(
        peer_two.peer_addr().unwrap(),
        [1u8; bt::PEER_ID_LEN].into(),
        [1u8; bt::INFO_HASH_LEN].into(),
        Extensions::new(),
    );

    // Add peer one to the manager
    manager.send(IPeerManagerMessage::AddPeer(peer_one_info, peer_one));

    // Check that peer one was added
    let response = manager.poll().unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            info!("PeerAdded\n1: {:?} \n=\n2: {:?}\n", peer_one_info, info)
        }
        _ => panic!("Unexpected First Peer Manager Response"),
    };


    // Remove peer one from the manager
    manager.send(IPeerManagerMessage::RemovePeer(peer_one_info));

    // Check that peer one was removed
    let response = manager.poll().unwrap();


    match response {
        OPeerManagerMessage::PeerRemoved(info) => {
            info!("PeerRemoved\n1: {:?} \n=\n2: {:?}\n", peer_one_info, info)
        }

        _ => panic!("Unexpected Third Peer Manager Response"),
    };

    // Try to add peer two, but make sure it goes through
    manager.send(IPeerManagerMessage::AddPeer(peer_two_info, peer_two));

    let response = manager.poll().unwrap();


    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            info!("PeerAdded\n1: {:?} \n=\n2: {:?}\n", peer_two_info, info)
        }

        _ => panic!("Unexpected Fourth Peer Manager Response"),
    };
}
