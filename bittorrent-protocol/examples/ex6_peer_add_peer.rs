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
use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use futures::sink::Sink;
use futures::stream::Stream;
use futures::sync::mpsc::{self, Receiver, Sender};
use futures::{future, AsyncSink, Future};
use futures::{Poll, StartSend};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::current_thread::Runtime;

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
    let mut runtime = Runtime::new().unwrap();

    let manager = PeerManagerBuilder::new()
    // Start logger
    init_log();
    info!("start run .......");

    let mut manager = PeerManagerBuilder::new()
        .with_peer_capacity(1)
        .build(runtime.handle());

    // Create two peers
    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let tcplisten=TcpListener::bind(&socket).unwrap();
    let listen_addr= tcplisten.local_addr().unwrap();
    let  peer_one  = TcpStream::connect(&listen_addr).wait().unwrap();

    let socket = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0);
    let tcplisten=TcpListener::bind(&socket).unwrap();
    let listen_addr= tcplisten.local_addr().unwrap();
    let peer_two  = TcpStream::connect(&listen_addr).wait().unwrap();

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
    let manager = runtime
        .block_on( manager.send(IPeerManagerMessage::AddPeer(peer_one_info, peer_one))
    ).unwrap();

    // Check that peer one was added
    let (response, mut manager) = runtime
        .block_on(
                 manager
                .into_future()
                .map(|(opt_item, stream)| (opt_item.unwrap(), stream))
                .map_err(|_| ()),
        )
        .unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            info!("PeerAdded\n1: {:?} \n=\n2: {:?}\n", peer_one_info, info)
        }
        _ => panic!("Unexpected First Peer Manager Response"),
    };

    // Try to add peer two, but make sure it was denied (start send returned not ready)
    let (response, manager) = runtime
        .block_on(
            future::lazy(move || {
                   future::ok::<_, ()>((
                   manager.start_send(IPeerManagerMessage::AddPeer(peer_two_info, peer_two)),
                   manager,
            ))
        })
    ).unwrap();

    let peer_two = match response {
        Ok(AsyncSink::NotReady(IPeerManagerMessage::AddPeer(info, peer_two))) => {
            println!("AddPeer\n1: {:?} \n=\n2: {:?}\n", peer_two_info, info);
            peer_two
        }
        _ => panic!("Unexpected Second Peer Manager Response"),
    };

    // Remove peer one from the manager
    let manager = runtime
        .block_on(
            manager
                .send(IPeerManagerMessage::RemovePeer(peer_one_info)))
        .unwrap();

    // Check that peer one was removed
    let (response, manager) = runtime
        .block_on(
            manager
                .into_future()
                .map(|(opt_item, stream)| (opt_item.unwrap(), stream))
                .map_err(|_| ()),
        ).unwrap();

    match response {
        OPeerManagerMessage::PeerRemoved(info) => {
            info!("PeerRemoved\n1: {:?} \n=\n2: {:?}\n", peer_one_info, info)
        }

        _ => panic!("Unexpected Third Peer Manager Response"),
    };

    // Try to add peer two, but make sure it goes through
    let manager = runtime
        .block_on(
            manager.send(IPeerManagerMessage::AddPeer(peer_two_info, peer_two))
        ).unwrap();

    let (response, _manager) = runtime
        .block_on(
            manager
                .into_future()
                .map(|(opt_item, stream)| (opt_item.unwrap(), stream))
                .map_err(|_| ()),
        )
        .unwrap();

    match response {
        OPeerManagerMessage::PeerAdded(info) => {
            info!("PeerAdded\n1: {:?} \n=\n2: {:?}\n", peer_two_info, info)
        }

        _ => panic!("Unexpected Fourth Peer Manager Response"),
    };
}
