
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

use bittorrent_protocol::handshake::transports::{TcpTransport,UtpTransport};
use bittorrent_protocol::handshake::{HandshakerManagerBuilder, InitiateMessage, Protocol, Extension, Extensions };
use btp_handshake::{Extension, Extensions, HandshakerManagerBuilder, InitiateMessage, Protocol};
use btp_handshake::transports::UtpTransport;
use btp_util::bt::InfoHash;

fn main() {

    // Start logger
    init_log();
    info!("start run .......");

    /**
     *   bittorrent-protocol/examples_data/file/music.zip  info-hash
     */
    let hash = InfoHash::from_hex("E5B6BECAFD04BA0A9B7BBE6883A86DEDA731AE3C");

    let addr = "127.0.0.1:44444".parse().expect(" socket parse error");

    // Show up as a uTorrent client...
    let peer_id = (*b"-UT2060-000000000000").into();

    let mut ext =Extensions::new();
    ext.add(Extension::ExtensionProtocol);

    let mut handshaker_manager = HandshakerManagerBuilder::new()
        .with_peer_id(peer_id)
        .with_extensions(ext)
        .build(UtpTransport)
        .unwrap();

    handshaker_manager.send(InitiateMessage::new(Protocol::BitTorrent, hash, addr)).unwrap();

    let completemessage = handshaker_manager.poll().unwrap();

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
