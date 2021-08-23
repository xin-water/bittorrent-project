
#[macro_use]
extern crate clap;

use hex;
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
use pendulum::future::TimerBuilder;
use pendulum::HashedWheelBuilder;
use std::fmt::Debug;
use std::fs::File;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use futures::future::{self, Either, Loop};
use futures::sink::Wait;
use futures::{Future, Sink, Stream};
use tokio::runtime::current_thread::{Runtime, Handle};
use tokio::io::AsyncRead;

use bittorrent_protocol::util::bt::{InfoHash, PeerId};

use bittorrent_protocol::dht::{DhtBuilder, DhtEvent, Handshaker as DhtHandshake, Router};

use bittorrent_protocol::peer::messages::builders::ExtendedMessageBuilder;
use bittorrent_protocol::peer::messages::{
    BitsExtensionMessage, PeerExtensionProtocolMessage, PeerWireProtocolMessage,
};
use bittorrent_protocol::peer::{
    IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder,
};

use bittorrent_protocol::select::discovery::{
    IDiscoveryMessage, ODiscoveryMessage, UtMetadataModule,
};
use bittorrent_protocol::select::{
    ControlMessage, IExtendedMessage, IUberMessage, OExtendedMessage, OUberMessage,
    UberModuleBuilder,
};
use bittorrent_protocol::handshake::{HandshakerManagerBuilder, Extensions, Extension, InitiateMessage, Protocol, HandshakerConfig};
use bittorrent_protocol::handshake::transports::{TcpTransport,UtpTransport};
use bittorrent_protocol::metainfo::Metainfo;

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

        /**
         *   bittorrent-protocol/examples_data/file/music.zip  info-hash
         */
        None => "E5B6BECAFD04BA0A9B7BBE6883A86DEDA731AE3C",
    };

    //let addr = matches.value_of("peer").unwrap().parse().unwrap();

    let output = match matches.value_of("output") {
        Some(s) => s,
        None => "./bittorrent-protocol/examples_data/download/metadata.torrent",
    };

    let hash: Vec<u8> = hex::decode(hash).unwrap();
    let info_hash = InfoHash::from_hash(&hash[..]).unwrap();

    let peer_id:PeerId = (*b"-UT2060-000000000000").into();
    let addr = "127.0.0.1:44444";

    // Create our main "core" event loop
    let mut runtime = Runtime::new().unwrap();
    let handle= runtime.handle();

    // Activate the extension protocol via the handshake bits
    let mut extensions = Extensions::new();
    extensions.add(Extension::ExtensionProtocol);

    info!("start component....");
    // Create a handshaker that can initiate connections with peers
    let (mut handshaker_send, mut handshaker_recv) = HandshakerManagerBuilder::new()
        .with_peer_id(peer_id)
        .with_extensions(extensions)
        .with_config(
            // Set a low handshake timeout so we dont wait on peers that arent listening on tcp
            HandshakerConfig::default().with_connect_timeout(Duration::from_millis(500)),
        )
        .build(TcpTransport,handle.clone())
        .unwrap()
        .into_parts();

    // Create a peer manager that will hold our peers and heartbeat/send messages to them
    let (mut peer_manager_send, mut peer_manager_recv) =
        PeerManagerBuilder::new().build(handle).into_parts();

    let timer = TimerBuilder::default().build(HashedWheelBuilder::default().build());
    let timer_recv = timer
        .sleep_stream(Duration::from_millis(100))
        .unwrap()
        .map(Either::B);
    let merged_recv = peer_manager_recv
        .map(Either::A)
        .map_err(|_| ())
        .select(timer_recv);

    // Create our UtMetadata selection module
    let (mut uber_send, mut uber_recv) = UberModuleBuilder::new()
                                       .with_extended_builder(Some(ExtendedMessageBuilder::new()))
                                       .with_discovery_module(UtMetadataModule::new())
                                       .build()
                                       .split();

    info!("commit DownloadMetainfo to uber_module....");
    // Tell the uber module we want to download metainfo for the given hash
    let uber_send = runtime.block_on(
        uber_send
            .send(IUberMessage::Discovery(
                IDiscoveryMessage::DownloadMetainfo(info_hash),
            ))
            .map_err(|_| ()),
    ).unwrap();

    ///////////////////////////////////////////////////////////////////////////////////

    // Hook up a future that feeds incoming (handshaken) peers over to the peer manager
    runtime.spawn(
        handshaker_recv
            .map_err(|_| ())
            .map(|complete_msg| {
                // Our handshaker finished handshaking some peer, get
                // the peer info as well as the peer itself (socket)

                let (_, extensions, hash, pid, addr, sock) = complete_msg.into_parts();

                // Only connect to peer that support the extension protocol...
                if extensions.contains(Extension::ExtensionProtocol) {

                      // Create our peer identifier used by our peer manager
                      let peer_info = PeerInfo::new(addr, pid, hash, extensions);

                      // Map to a message that can be fed to our peer manager
                      IPeerManagerMessage::AddPeer(peer_info, sock)
                } else {
                      panic!("Chosen Peer Does Not Support Extended Messages")
                }
            })
            .forward(peer_manager_send.clone().sink_map_err(|_| ()))
            .map(|_| ()),
    );

    // Hook up a future that receives messages from the peer manager
    runtime.spawn(future::loop_fn(
        (merged_recv, info_hash, uber_send.sink_map_err(|_| ())),
        |(merged_recv, info_hash, select_send)| {
            merged_recv
                .into_future()
                .map_err(|_| ())
                .and_then(move |(opt_item, merged_recv)| {
                    let opt_message = match opt_item.unwrap() {
                        Either::A(OPeerManagerMessage::PeerAdded(info)) => {
                            info!("[peer_manage_steam_recv] PeerAdded -- Connected To Peer: {:?}\n", info);
                            Some(IUberMessage::Control(ControlMessage::PeerConnected(info)))
                        }

                        Either::A(OPeerManagerMessage::PeerRemoved(info)) => {
                            info!("[peer_manage_steam_recv] PeerRemoved {:?} \n", info);
                            Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                                info,
                            )))
                        }

                        Either::A(OPeerManagerMessage::PeerDisconnect(info)) => {
                            info!("[peer_manage_steam_recv] PeerDisconnect {:?} \n", info);
                            Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                                info,
                            )))
                        }

                        Either::A(OPeerManagerMessage::PeerError(info, error)) => {
                            info!("[peer_manage_steam_recv] PeerError {:?} \n--msg: {:?}\n", info, error);
                            Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                                info,
                            )))
                        }

                        Either::A(OPeerManagerMessage::ReceivedMessage(
                                      info,
                                      PeerWireProtocolMessage::BitField(bytes))) => {
                            info!("[peer_manage_steam_recv] BitField : {:?}\n", bytes);
                            None
                        }

                        Either::A(OPeerManagerMessage::ReceivedMessage(
                                      info,
                                      PeerWireProtocolMessage::BitsExtension(BitsExtensionMessage::Extended(
                                                                                 extended,
                                                                             )),
                                  )) => {
                            info!("[peer_manage_steam_recv] BitsExtension : \n msg:{:?}\n msg_size:{:?}", &extended,extended.bencode_size());
                            Some(IUberMessage::Extended(
                                IExtendedMessage::RecievedExtendedMessage(info, extended),
                            ))
                        }

                        Either::A(OPeerManagerMessage::ReceivedMessage(
                                      info,
                                      PeerWireProtocolMessage::ProtExtension(
                                          PeerExtensionProtocolMessage::UtMetadata(message),
                                      ),
                                  )) => {
                            info!("[peer_manage_steam_recv] UtMetadata : {:?}\n", &message.message_size());
                            Some(IUberMessage::Discovery(
                                IDiscoveryMessage::ReceivedUtMetadataMessage(info, message),
                            ))
                        }

                        Either::B(_) => Some(IUberMessage::Control(ControlMessage::Tick(
                            Duration::from_millis(100),
                        ))),
                        _ => None,
                    };

                    match opt_message {
                        Some(message) => {
                            Either::A(select_send.send(message).map(move |select_send| {
                                Loop::Continue((merged_recv, info_hash, select_send))
                            }))
                        }
                        None => Either::B(future::ok(Loop::Continue((
                            merged_recv,
                            info_hash,
                            select_send,
                        )))),
                    }
                })
        },
    ));

    /////////////////////////////////////////////////////////////////////////////////

    // Send the peer given from the command line over to the handshaker to initiate a connection
    runtime.block_on(
        handshaker_send
            .send(InitiateMessage::new(
                Protocol::BitTorrent,
                info_hash,
                addr.parse().unwrap(),
            ))
            .map_err(|_| ()),
    ).unwrap();


    let metainfo = runtime.block_on(
        future::loop_fn(
            (uber_recv, peer_manager_send.sink_map_err(|_| ()), None),
            |(select_recv, map_peer_manager_send, mut opt_metainfo)| {
                select_recv.into_future().map_err(|_| ()).and_then(
                    move |(opt_message, select_recv)| {
                        let opt_message = opt_message.and_then(|message| match message {
                            OUberMessage::Extended(OExtendedMessage::SendExtendedMessage(
                                                       info,
                                                       ext_message,
                                                   )) => {
                                info!(
                                    "[select_recv] OExtendedMessage::SendExtendedMessage: \n\
                                    --info:{:?} \n\
                                    --msg: {:?} \n",
                                    info,
                                    ext_message
                                );
                                Some(IPeerManagerMessage::SendMessage(
                                    info,
                                    0,
                                    PeerWireProtocolMessage::BitsExtension(
                                        BitsExtensionMessage::Extended(ext_message),
                                    ),
                                ))
                            }
                            OUberMessage::Discovery(ODiscoveryMessage::SendUtMetadataMessage(
                                                        info,
                                                        message,
                                                    )) => {
                                info!(
                                    "[select_recv] ODiscoveryMessage::SendUtMetadataMessage \n\
                                    --info: {:?} \n\
                                    --msg: {:?}  \n",
                                    info,
                                    message
                                );
                                Some(IPeerManagerMessage::SendMessage(
                                    info,
                                    0,
                                    PeerWireProtocolMessage::ProtExtension(
                                        PeerExtensionProtocolMessage::UtMetadata(message),
                                    ),
                                ))
                            }
                            OUberMessage::Discovery(ODiscoveryMessage::DownloadedMetainfo(
                                                        metainfo,
                                                    )) => {
                                info!("[select_recv]  ODiscoveryMessage::DownloadedMetainfo\n");
                                opt_metainfo = Some(metainfo);
                                None
                            }
                            _ => panic!("[select_recv] Unexpected Message For Uber Module..."),
                        });

                        match (opt_message, opt_metainfo.take()) {
                            (Some(message), _) => {
                                Either::A(map_peer_manager_send.send(message).map(
                                    move |peer_manager_send| {
                                        Loop::Continue((
                                            select_recv,
                                            peer_manager_send,
                                            opt_metainfo,
                                        ))
                                    },
                                ))
                            }
                            (None, None) => Either::B(future::ok(Loop::Continue((
                                select_recv,
                                map_peer_manager_send,
                                opt_metainfo,
                            )))),
                            (None, Some(metainfo)) => Either::B(future::ok(Loop::Break(metainfo))),
                        }
                    },
                )
            },
        ))
        .unwrap();

    // Write the metainfo file out to the user provided path
    File::create(output)
        .unwrap()
        .write_all(&metainfo.to_bytes())
        .unwrap();
    info!("种子文件下载完成！\npath:{:?}", output);
}
