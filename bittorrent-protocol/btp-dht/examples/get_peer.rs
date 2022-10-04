use btp_dht::{DhtBuilder, Handshaker, Router};
use btp_util::bt::{InfoHash, PeerId};
use log::{info, LevelFilter};
use std::collections::HashSet;
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::thread::{self};
use log4rs::append::console::{ConsoleAppender, Target};
use log4rs::Config;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;

fn main() {
    // Start logger
    init_log();
    info!("start run .......");

    let dht = DhtBuilder::new()
        .add_router(Router::BitTorrent)
        .add_router(Router::Custom(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(84,68,129,186),6881))))
        .add_router(Router::Custom(SocketAddr::V4("24.38.230.49:50321".parse().unwrap())))
        .set_source_addr(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(0, 0, 0, 0),
            6889,
        )))
        .set_announce_port(5432)
        .set_read_only(false)
        .start_mainline()
        .unwrap();

    // Spawn a thread to listen to and report events
    let events = dht.events();
    thread::spawn(move || {
        for event in events {
            println!("\nReceived Dht Event {:?}", event);
        }
    });

    // let hash = InfoHash::from_bytes(b"My Unique Info Hash");
    //  ubuntu-22.04.1-desktop-amd64.iso  is  3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0
    let hash = InfoHash::from_bytes(b"3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0");
    println!("\n InfoHash is: {:?}", &hash);
    // Let the user announce or search on our info hash
    let stdin = io::stdin();
    let stdin_lock = stdin.lock();
    for byte in stdin_lock.bytes() {
       let rx= match &[byte.unwrap()] {
            b"a" => dht.search(hash.into(), true),
            b"s" => dht.search(hash.into(), false),
            _ => None,
        };

       if let Some(rx) = rx {
           let mut total = 0;
           for addr in rx.recv() {
               total += 1;
               println!("Received new peer {:?}, total unique peers {:?}",addr,total);
           }
       }
    }
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
                .build(LevelFilter::Warn),
        )
        .unwrap();

    let _ = log4rs::init_config(config).unwrap();
}
