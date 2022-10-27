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
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use btp_handshake::Extensions;
use btp_peer::{IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder};
use btp_util::bt;


#[tokio::main]
async fn main() {

    // Start logger
    init_log();
    info!("start run .......");

    let mut manager = PeerManagerBuilder::new()
        .with_peer_capacity(1)
        .build();

    // Create two peers
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let tcplisten=TcpListener::bind(&socket).await.unwrap();
    let listen_addr= tcplisten.local_addr().unwrap();

    //模拟远程peer stream
    let mut peer_stream = TcpStream::connect(&listen_addr).await.unwrap();


    // 模拟远程peer信息
    let peer_info = PeerInfo::new(
        peer_stream.peer_addr().unwrap(),
        [0u8; bt::PEER_ID_LEN].into(),
        [0u8; bt::INFO_HASH_LEN].into(),
        Extensions::new(),
    );

    // Add peer one to the manager
    manager.send(IPeerManagerMessage::AddPeer(peer_info, peer_stream)).await;

    // Check that peer one was added
    let response = manager.poll().await.unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            info!("PeerAdded \n1: {:?} \n2: {:?}\n", peer_info, info)
        }
        _ => panic!("Unexpected First Peer Manager Response"),
    };


    // Remove peer one from the manager
    manager.send(IPeerManagerMessage::RemovePeer(peer_info)).await;

    // Check that peer one was removed
    let response = manager.poll().await.unwrap();


    match response {
        OPeerManagerMessage::PeerRemoved(info) => {
            info!("PeerRemoved \n1: {:?} \n2: {:?}\n", peer_info, info)
        }

        _ => panic!("Unexpected Third Peer Manager Response"),
    };
}


fn init_log() {
    let stdout = ConsoleAppender::builder()
        .target(Target::Stdout)
        .encoder(Box::new(PatternEncoder::new(
            "[Console] {d} - {l} -{t} - {m}{n}",
        )))
        .build();


    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .build(
            Root::builder()
                .appender("stdout")
                .build(LevelFilter::Trace),
        )
        .unwrap();

    let _ = log4rs::init_config(config).unwrap();
}
