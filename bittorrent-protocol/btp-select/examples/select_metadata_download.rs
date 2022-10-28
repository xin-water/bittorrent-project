
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

use std::fmt::Debug;
use std::fs::File;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{mpsc, Arc, Mutex};
use tokio::net::TcpStream;

use btp_handshake::{Extension, Extensions, HandshakerConfig, HandshakerManagerBuilder, HandshakerManagerSink, HandshakerManagerStream, InfoHash, InHandshake, InitiateMessage, PeerId, Protocol};
use btp_handshake::transports::TcpTransport;
use btp_metainfo::Metainfo;
use btp_peer::messages::builders::ExtendedMessageBuilder;
use btp_peer::{IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder, PeerManagerSink, PeerManagerStream};
use btp_peer::messages::{BitsExtensionMessage, PeerExtensionProtocolMessage, PeerWireProtocolMessage};
use btp_select::discovery::{IDiscoveryMessage, ODiscoveryMessage, UtMetadataModule};
use btp_select::{ControlMessage, IExtendedMessage, IUberMessage, OExtendedMessage, OUberMessage, UberModule, UberModuleBuilder};

#[tokio::main]
async fn main() {

    // Start logger
    init_log();
    info!("start run .......");

    let (mut handshaker_send, handshaker_recv, mut peer_manager_send, peer_manager_recv, uber_module) = create_component().await;


    let task1=tokio::spawn(handshaker_rx_handler(handshaker_recv, peer_manager_send.clone()));

    let task2=tokio::spawn(peer_rx_handler(peer_manager_recv, uber_module.clone()));

    let task3=tokio::spawn(select_tick(uber_module.clone()));

    // 离谱，在这会卡死，发送拓展给peer的时候，peer入站一直收不到消息，通道有消息也会阻塞？
    // 放当前协程最后执行又不会，想不通
    //let task4=tokio::spawn(select_rx_handler(uber_module.clone(),peer_manager_send));


    ///////////////////////////////////////////////////////////////////////////////////////////////////////

    /**
    *   bittorrent-protocol/btp-disk/torrent/music.torrent  info-hash
    */
    let info_hash = InfoHash::from_hex("E5B6BECAFD04BA0A9B7BBE6883A86DEDA731AE3C");

    // 本地启动一个bt客户端，比如qb, 用来测试
    let addr = "127.0.0.1:44444";

    {
        info!("commit DownloadMetainfo to uber_module....");
        uber_module.lock().unwrap()
            .send(IUberMessage::Discovery(IDiscoveryMessage::DownloadMetainfo(info_hash)))
            .expect("uber_module send msg: DownloadMetainfo fail");
    }

    info!("commit peer handshake ....");
    handshaker_send.send(
        InHandshake::Init(
            InitiateMessage::new(
            Protocol::BitTorrent,
            info_hash,
            addr.parse().unwrap())
        )
    ).await.unwrap();

    select_rx_handler(uber_module.clone(),peer_manager_send.clone()).await;
    handshaker_send.send(InHandshake::Shutdown).await;
    peer_manager_send.send(IPeerManagerMessage::Shutdown).await;
    //uber_module.lock().unwrap().send()
    task1.await;
    task2.await;
    //永久循环的，不等它
    //task3.await;
    //task4.await;
    println!("run end")
}

async fn create_component() -> (HandshakerManagerSink, HandshakerManagerStream<TcpStream>, PeerManagerSink<TcpStream>, PeerManagerStream, Arc<Mutex<UberModule>>) {
    info!("start component....");

    let peer_id: PeerId = (*b"-UT2060-000000000000").into();

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
        .await
        .unwrap()
        .into_parts();

    // Create a peer manager that will hold our peers and heartbeat/send messages to them
    let (mut peer_manager_send, mut peer_manager_recv) =
        PeerManagerBuilder::new().build().into_parts();


    // Create our UtMetadata selection module
    let mut uber_module = Arc::new(Mutex::new(UberModuleBuilder::new()
        .with_extended_builder(Some(ExtendedMessageBuilder::new()))
        .with_discovery_module(UtMetadataModule::new())
        .build()));
    (handshaker_send, handshaker_recv, peer_manager_send, peer_manager_recv, uber_module)
}

async fn select_rx_handler(uber_module: Arc<Mutex<UberModule>>,mut peer_manager_send: PeerManagerSink<TcpStream>, ) {
    loop {
        // info!("[select_rx_handler] in work");

        // 使用大括号限定作用域, 释放锁.
        let message = {
            uber_module.lock().unwrap().poll().unwrap()
        };

        //info!("[select_rx_handler] :{:?}",&message);

        if let Some(message) = message {
            match message {
                OUberMessage::Extended(OExtendedMessage::SendExtendedMessage(info, ext_message)) => {
                    info!(
                        "[select_rx_handler] SendExtendedMessage:\n--peer_info: {:?}\n--msg: {:?}\n",
                        info.addr(), ext_message
                    );

                    peer_manager_send.send(IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::BitsExtension(
                            BitsExtensionMessage::Extended(ext_message),
                        ),
                    )).await.expect("select_rx_handler SendExtendedMessage");
                }

                OUberMessage::Discovery(ODiscoveryMessage::SendUtMetadataMessage(info, message)) => {
                    info!(
                        "[select_rx_handler] SendUtMetadataMessage \n--peer_info: {:?} \n--msg: {:?}\n",
                        info.addr(), message
                    );
                    peer_manager_send.send(IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::ProtExtension(
                            PeerExtensionProtocolMessage::UtMetadata(message),
                        ),
                    )).await.expect("select_rx_handler SendUtMetadataMessage");
                }
                OUberMessage::Discovery(ODiscoveryMessage::DownloadedMetainfo(metainfo)) => {
                    info!("[select_rx_handler]  DownloadedMetainfo\n");

                    // Write the metainfo file out to the user provided path
                    File::create("./bittorrent-protocol/btp-select/download/metadata.torrent")
                        .unwrap()
                        .write_all(&metainfo.to_bytes())
                        .unwrap();
                    info!("种子文件下载完成！\npath:{:?}", "bittorrent-protocol/btp-select/download/metadata.torrent");
                    return;
                }
                _ => panic!("[select_rx_handler] Unexpected Message For Uber Module..."),
            }
        }
    }
}

async fn select_tick(mut uber_module_clone: Arc<Mutex<UberModule>>){
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;

            let message = IUberMessage::Control(ControlMessage::Tick(
                Duration::from_millis(100),
            ));

            {
                uber_module_clone.lock().unwrap().send(message).unwrap();
            }
        }

}

async fn peer_rx_handler(mut peer_manager_recv: PeerManagerStream, mut uber_module_clone: Arc<Mutex<UberModule>>) {
    loop {
        //info!("[peer_rx_handler] in work");
        let item = peer_manager_recv.rx().await.expect("peer_manager_recv break");
        info!("[peer_rx_handler] opeer_manager_msg {:?} \n", &item);

        let opt_message = match item {
            OPeerManagerMessage::PeerAdded(info) => {
                Some(IUberMessage::Control(ControlMessage::PeerConnected(info)))
            }

            OPeerManagerMessage::PeerRemoved(info) => {
                Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                    info,
                )))
            }

            OPeerManagerMessage::PeerDisconnect(info) => {
                Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                    info,
                )))
            }

            OPeerManagerMessage::PeerError(info, _error) => {
                Some(IUberMessage::Control(ControlMessage::PeerDisconnected(
                    info,
                )))
            }

            OPeerManagerMessage::ReceivedMessage(
                info,
                PeerWireProtocolMessage::BitsExtension(
                    BitsExtensionMessage::Extended(extended))) => {
                Some(IUberMessage::Extended(
                    IExtendedMessage::RecievedExtendedMessage(info, extended),
                ))
            }

            OPeerManagerMessage::ReceivedMessage(
                info,
                PeerWireProtocolMessage::ProtExtension(
                    PeerExtensionProtocolMessage::UtMetadata(message))) => {
                Some(IUberMessage::Discovery(
                    IDiscoveryMessage::ReceivedUtMetadataMessage(info, message),
                ))
            }

            _ => None,
        };

        match opt_message {
            Some(message) => {
                uber_module_clone.lock().unwrap().send(message).unwrap();
            }
            None => {}
        }
    }
}

async fn handshaker_rx_handler(mut handshaker_recv: HandshakerManagerStream<TcpStream>, mut peer_manager_send: PeerManagerSink<TcpStream>) {

    info!("[handshake_rx_handler] in work");
    let (_, extensions, hash, pid, addr, sock) = handshaker_recv.poll().await.expect("handshaker_recv break").into_parts();

    // Only connect to peer that support the extension protocol...
    if extensions.contains(Extension::ExtensionProtocol) {

        info!("[handshaker_rx_handler] AddPeer: {:?} to peer_manmager_module....",pid);

        // Create our peer identifier used by our peer manager
        let peer_info = PeerInfo::new(addr, pid, hash, extensions);

        // Map to a message that can be fed to our peer manager
        peer_manager_send.send(IPeerManagerMessage::AddPeer(peer_info, sock)).await.expect(" handshake peer_manager_send fail");
    } else {
        panic!("Chosen Peer Does Not Support Extended Messages")
    }
}


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
