#[macro_use]
extern crate log;

#[macro_use]
extern crate clap;

use hex;
use log::{LogLevel, LogLevelFilter, LogMetadata, LogRecord};
use pendulum::future::TimerBuilder;
use pendulum::HashedWheelBuilder;
use std::fmt::Debug;
use std::fs::File;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{mpsc, Arc, Mutex};

use bittorrent_protocol::util::bt::{InfoHash, PeerId};

use bittorrent_protocol::dht::{DhtBuilder, DhtEvent, Handshaker as DhtHandshake, Router};

use bittorrent_protocol::peer::messages::builders::ExtendedMessageBuilder;
use bittorrent_protocol::peer::messages::{
    BitsExtensionMessage, PeerExtensionProtocolMessage, PeerWireProtocolMessage,
};
use bittorrent_protocol::peer::{
    IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder,
};

use bittorrent_protocol::select::ut_metadata::{
    IUtMetadataMessage, OUtMetadataMessage, UtMetadataModule,
};
use bittorrent_protocol::select::{
    ControlMessage, IExtendedMessage, IUberMessage, OExtendedMessage, OUberMessage,
    UberModuleBuilder,
};
use bittorrent_protocol::handshake::{HandshakerManagerBuilder, Extensions, Extension, InitiateMessage, Protocol, HandshakerConfig};
use bittorrent_protocol::handshake::transports::TcpTransport;
use bittorrent_protocol::metainfo::Metainfo;

fn main() {
    log::set_logger(|m| {
        m.set(LogLevelFilter::max());
        Box::new(SimpleLogger)
    }).unwrap();

    let matches = clap_app!(myapp =>
        (version: "1.0")
        (author: "Andrew <amiller4421@gmail.com>")
        (about: "Download torrent file from info hash")
        (@arg hash: -h  +takes_value "InfoHash of the torrent")
        //(@arg peer: -p +required +takes_value "Single peer to connect to of the form addr:port")
        (@arg output: -f +takes_value "Output to write the torrent file to")
    )
    .get_matches();

    let hash = match matches.value_of("hash") {
        Some(s) => s,

        //ubuntu.torrent  hash:  magnet:?xt=urn:btih:d1101a2b9d202811a05e8c57c557a20bf974dc8a
        None => "9B47016CFC165D39923ADDAF3A55FC1F90E0BA95",
    };

    //let addr = matches.value_of("peer").unwrap().parse().unwrap();

    let output = match matches.value_of("output") {
        Some(s) => s,
        None => "./bittorrent-protocol/examples_data/download/metadata.torrent",
    };

    let hash: Vec<u8> = hex::decode(hash).unwrap();
    let info_hash = InfoHash::from_hash(&hash[..]).unwrap();

    let peer_id = (*b"-UT2060-000000000000").into();

    // Activate the extension protocol via the handshake bits
    let mut extensions = Extensions::new();
    extensions.add(Extension::ExtensionProtocol);

    // Create a handshaker that can initiate connections with peers
    let (mut handshaker_send, mut handshaker_recv) = HandshakerManagerBuilder::new()
        .with_peer_id(peer_id)
        .with_extensions(extensions)
        .with_config(
            // Set a low handshake timeout so we dont wait on peers that arent listening on tcp
            HandshakerConfig::default().with_connect_timeout(Duration::from_millis(500)),
        )
        .build(TcpTransport)
        .unwrap()
        .into_parts();

    // Create a peer manager that will hold our peers and heartbeat/send messages to them
    let (mut peer_manager_send, mut peer_manager_recv) =
        PeerManagerBuilder::new().build().into_parts();


    // Create our UtMetadata selection module
    let mut uber_module = Arc::new(Mutex::new(UberModuleBuilder::new()
                                       .with_extended_builder(Some(ExtendedMessageBuilder::new()))
                                       .with_ut_metadata_module(UtMetadataModule::new())
                                       .build()));


    /////////////////////////////////////////////////////////////////////////////////////////////////////////

    // Hook up a future that feeds incoming (handshaken) peers over to the peer manager
    let mut handshark_peer_manager_send = peer_manager_send.clone();
    std::thread::spawn(move ||{

        let (_, extensions, hash, pid, addr, sock) = handshaker_recv.poll().unwrap().into_parts();

        // Only connect to peer that support the extension protocol...
        if extensions.contains(Extension::ExtensionProtocol) {

            // Create our peer identifier used by our peer manager
            let peer_info = PeerInfo::new(addr, pid, hash, extensions);

            // Map to a message that can be fed to our peer manager
            handshark_peer_manager_send.send( IPeerManagerMessage::AddPeer(peer_info, sock));
        } else {
            panic!("Chosen Peer Does Not Support Extended Messages")
        }

    });

    // Hook up a future that receives messages from the peer manager
    let mut uber_module_clone = uber_module.clone();
     std::thread::spawn(move ||{
         loop {
             let opt_item=peer_manager_recv.poll().unwrap();

             let opt_message = match opt_item {
                 OPeerManagerMessage::PeerAdded(info) => {
                     println!("[merged_recv] PeerAdded -- Connected To Peer: {:?}\n", info);
                     Some(IUberMessage::Control(ControlMessage::PeerConnected(info)))
                 }

                 OPeerManagerMessage::PeerRemoved(info) => {
                     println!("[merged_recv] PeerRemoved {:?} \n", info);
                     Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                         info,
                     )))
                 }

                 OPeerManagerMessage::PeerDisconnect(info) => {
                     println!("[merged_recv] PeerDisconnect {:?} \n", info);
                     Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                         info,
                     )))
                 }

                 OPeerManagerMessage::PeerError(info, error) => {
                     println!("[merged_recv] PeerError {:?} \n--msg: {:?}\n", info, error);
                     Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                         info,
                     )))
                 }

                 OPeerManagerMessage::ReceivedMessage(
                               info,
                               PeerWireProtocolMessage::BitsExtension(
                                   BitsExtensionMessage::Extended(extended))) => {
                     println!("[merged_recv] BitsExtension : {:?}\n", &extended);
                     Some(IUberMessage::Extended(
                         IExtendedMessage::RecievedExtendedMessage(info, extended),
                     ))
                 }

                 OPeerManagerMessage::ReceivedMessage(
                               info,
                               PeerWireProtocolMessage::ProtExtension(
                                   PeerExtensionProtocolMessage::UtMetadata(message))) => {
                     println!("[merged_recv] UtMetadata : {:?}\n", &message.message_size());
                     Some(IUberMessage::Ut_Metadata(
                         IUtMetadataMessage::ReceivedUtMetadataMessage(info, message),
                     ))
                 }

                 _ => None,
             };

             match opt_message {
                 Some(message) => {
                     uber_module_clone.lock().unwrap().send(message).unwrap();
                 }
                 None =>{}
             }
         }
     });

    let mut uber_module_clone = uber_module.clone();
    std::thread::spawn(move ||{
        loop {

            std::thread::sleep(Duration::from_millis(100));

            let message = IUberMessage::Control(ControlMessage::Tick(
                 Duration::from_millis(100),
             ));

            uber_module_clone.lock().unwrap().send(message).unwrap();
        }
    });

    ///////////////////////////////////////////////////////////////////////////////////////////////////////

    // Tell the uber module we want to download metainfo for the given hash
    uber_module.lock().unwrap()
        .send(IUberMessage::Ut_Metadata(IUtMetadataMessage::DownloadMetainfo(info_hash)))
        .expect("uber_module send msg: DownloadMetainfo fail");

    handshaker_send.send(
        InitiateMessage::new(
            Protocol::BitTorrent,
            info_hash,
            "127.0.0.1:44444".parse().unwrap()
        )
    ).unwrap();


    let mut opt_metainfo :Option<Metainfo>= None;
    loop {

        let message = uber_module.lock().unwrap().poll().unwrap();

        let opt_message = message.and_then(|message|
            match message {
                OUberMessage::Extended(OExtendedMessage::SendExtendedMessage(info, ext_message)) => {
                    println!(
                        "[select_recv] SendExtendedMessage --peer_info: {:?}\n--msg: {:?}\n",
                        info, ext_message
                    );

                    Some(IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::BitsExtension(
                            BitsExtensionMessage::Extended(ext_message),
                        ),
                    ))
                }

                OUberMessage::Ut_Metadata(OUtMetadataMessage::SendUtMetadataMessage(info, message)) => {
                    println!(
                        "[select_recv] SendUtMetadataMessage --peer_info: {:?} \n--msg: {:?}\n",
                        info, message
                    );
                    Some(IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::ProtExtension(
                            PeerExtensionProtocolMessage::UtMetadata(message),
                        ),
                    ))
                }

                OUberMessage::Ut_Metadata(OUtMetadataMessage::DownloadedMetainfo(metainfo)) => {
                    println!("[select_recv]  DownloadedMetainfo\n");
                    opt_metainfo = Some(metainfo);
                    None
                }
                _ => panic!("[select_recv] Unexpected Message For Uber Module..."),
        });

        match (opt_message, opt_metainfo.take()) {
            (Some(message), _) => {
                peer_manager_send.send(message);
            }
            (None, None) => {

            }
            (None, Some(metainfo)) =>{
                // Write the metainfo file out to the user provided path
                File::create(output)
                    .unwrap()
                    .write_all(&metainfo.to_bytes())
                    .unwrap();
                println!("种子文件下载完成！\npath:{:?}", output);
                break
            }
        }
    }
}

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        metadata.level() <= LogLevel::Info
    }

    fn log(&self, record: &LogRecord) {
        if self.enabled(record.metadata()) {
            println!("{:?} - {:?}", record.level(), record.args());
        }
    }
}
