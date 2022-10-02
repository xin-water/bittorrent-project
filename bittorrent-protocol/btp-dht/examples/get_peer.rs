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

    let handshaker = SimpleHandshaker {
        filter: HashSet::new(),
        count: 0,
    };
    let dht = DhtBuilder::new()
        .add_router(Router::BitTorrent)
        .add_router(Router::Custom(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(84,68,129,186),6881))))
        .add_router(Router::Custom(SocketAddr::V4("24.38.230.49:50321".parse().unwrap())))
        .set_source_addr(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(0, 0, 0, 0),
            6889,
        )))
        .set_read_only(false)
        .start_mainline(handshaker)
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

    // Let the user announce or search on our info hash
    let stdin = io::stdin();
    let stdin_lock = stdin.lock();
    for byte in stdin_lock.bytes() {
        match &[byte.unwrap()] {
            b"a" => dht.search(hash.into(), true),
            b"s" => dht.search(hash.into(), false),
            _ => (),
        }
    }
}

struct SimpleHandshaker {
    filter: HashSet<SocketAddr>,
    count: usize,
}

impl Handshaker for SimpleHandshaker {
    /// Type of stream used to receive connections from.
    type Metadata = ();

    /// Unique peer id used to identify ourselves to other peers.
    fn id(&self) -> PeerId {
        [0u8; 20].into()
    }

    /// Advertise port that is being listened on by the handshaker.
    ///
    /// It is important that this is the external port that the peer will be sending data
    /// to. This is relevant if the client employs nat traversal via upnp or other means.
    fn port(&self) -> u16 {
        6889
    }

    /// Initiates a handshake with the given socket address.
    fn connect(&mut self, _: Option<PeerId>, _: InfoHash, addr: SocketAddr) {
        if self.filter.contains(&addr) {
            return;
        }

        self.filter.insert(addr);
        self.count += 1;
        println!(
            "Received new peer {:?}, total unique peers {:?}",
            addr, self.count
        );
    }

    /// Send the given Metadata back to the client.
    fn metadata(&mut self, _: Self::Metadata) {
        ()
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
                .build(LevelFilter::Info),
        )
        .unwrap();

    let _ = log4rs::init_config(config).unwrap();
}
