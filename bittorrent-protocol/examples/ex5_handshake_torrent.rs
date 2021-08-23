
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
use std::fs::File;

use std::io::{self, BufRead, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::thread;
use std::time::Duration;

use futures::{Future, Sink, Stream};
use tokio::runtime::current_thread::{Runtime, Handle};
use bittorrent_protocol::handshake::transports::{TcpTransport,UtpTransport};
use bittorrent_protocol::handshake::{HandshakerManagerBuilder, InitiateMessage, Protocol, Extension, Extensions };

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

    // let mut stdout = io::stdout();
    // let stdin = io::stdin();
    // let mut lines = stdin.lock().lines();
    //
    // stdout.write(b"Enter An InfoHash In Hex Format: ").unwrap();
    // stdout.flush().unwrap();
    //
    // let hex_hash = lines.next().unwrap().unwrap();

    /**
     *   bittorrent-protocol/examples_data/file/music.zip  info-hash
     */
    let hex_hash = "E5B6BECAFD04BA0A9B7BBE6883A86DEDA731AE3C";

    let hash = hex_to_bytes(&hex_hash).into();

    // stdout.write(b"Enter An Address And Port (eg: addr:port): ").unwrap();
    // stdout.flush().unwrap();
    //
    // let str_addr = lines.next().unwrap().unwrap();
    let str_addr = "127.0.0.1:44444";
    let addr = str_to_addr(&str_addr);

    let mut runtime = Runtime::new().unwrap();
    let handle= runtime.handle();

    // Show up as a uTorrent client...
    let peer_id = (*b"-UT2060-000000000000").into();

    let mut ext =Extensions::new();
    ext.add(Extension::ExtensionProtocol);
    let (handshaker_manager_sink, handshaker_manager_steam) = HandshakerManagerBuilder::new()
        .with_peer_id(peer_id)
        .with_extensions(ext)
        .build(TcpTransport,handle)
        .unwrap()
        .into_parts();

    runtime.block_on(handshaker_manager_sink.send(InitiateMessage::new(Protocol::BitTorrent, hash, addr))).unwrap();

    let completemessage = runtime.block_on(
        handshaker_manager_steam
                .into_future()
                .map(|(opt_peer, _)| opt_peer.unwrap())
        )
        .unwrap_or_else(|_| panic!(""));

    let (pro,ext,hash, peer_id,addr,s) = completemessage.into_parts();

    println!("pro:{:?}\n\
              ext:{:?}\n\
              hash:{:?}\n\
              peer_id:{:?}\n\
              addr:{:?}\n",
             pro,ext,hex::encode(hash),String::from_utf8_lossy(peer_id.as_ref()),addr
    );
    println!("Connection With Peer Established...Closing In 10 Seconds");
    thread::sleep(Duration::from_secs(10));

}


fn hex_to_bytes(hex: &str) -> [u8; 20] {
    let mut exact_bytes = [0u8; 20];

    for byte_index in 0..20 {
        let high_index = byte_index * 2;
        let low_index = (byte_index * 2) + 1;

        let hex_chunk = &hex[high_index..low_index + 1];
        let byte_value = u8::from_str_radix(hex_chunk, 16).unwrap();

        exact_bytes[byte_index] = byte_value;
    }

    exact_bytes
}

fn str_to_addr(addr: &str) -> SocketAddr {
    addr.to_socket_addrs().unwrap().next().unwrap()
}
