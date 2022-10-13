use btp_dht::{DhtBuilder, MainlineDht, Router};
use btp_util::bt::InfoHash;
use log::{error, info, LevelFilter};
use std::io::{self, Read};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use log4rs::append::console::{ConsoleAppender, Target};
use log4rs::Config;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use tokio::sync::mpsc::Receiver;

#[tokio::main]
async fn main() {
    // Start logger
    init_log();
    info!("start run .......");

    let dht = MainlineDht::builder()
        .add_defalut_router()
        .add_nodes_str(["24.38.230.49:50321","84.68.129.186:6881"])
        .set_run_port(6889)
        .set_announce_port(5432)
        .set_read_only(false)
        .start_mainline()
        .await
        .unwrap();

    // Spawn a thread to listen to and report events
    let  events = dht.events();
    tokio::spawn(async move{
        if let Some(mut events) = events{

            while let Some(event) = events.recv().await {
                println!("\nReceived Dht Event {:?}", event);
            }

        }
    });

    // let hash = InfoHash::from_bytes(b"My Unique Info Hash");

    //  ubuntu-22.04.1-desktop-amd64.iso  is  3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0

    // let bytes= hex::decode("3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0").unwrap();
    // let hash = InfoHash::from_hash(&bytes).unwrap();

    let hash = InfoHash::from_hex("3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0");

    if !dht.bootstrapped().await {
        log::error!("dht 启动失败，程序退出！");
        return;
    }
    println!(">> ");
    println!("    s    search for the specified ubuntu-22.04.1-desktop-amd64.iso");
    println!("    a    announce the specified ubuntu-22.04.1-desktop-amd64.iso");
    println!("    l    list dht value");
    println!("    q    quit");


    // Let the user announce or search on our info hash
    let stdin = io::stdin();
    let stdin_lock = stdin.lock();
    for byte in stdin_lock.bytes() {

        match &[byte.unwrap()] {
            b"a" => {
                println!("\n InfoHash is: {:?}", &hash);
                if let Some(rx) = dht.search(hash.into(), true).await{
                    tokio::spawn(print_peer(rx));
                }
            },
            b"s" => {
                println!("\n InfoHash is: {:?}", &hash);
                if let Some(rx) = dht.search(hash.into(), false).await{
                    tokio::spawn(print_peer(rx));
                }
            },
            b"l" => {
                if let Some(v) = dht.getValues().await{
                   println!("{:?}",v)
                }
            },
            b"q" => return,

            _ => (),
        };

    }
}

async fn print_peer(mut rx: Receiver<SocketAddr>){
    let mut total = 0;

    while let Some(addr) = rx.recv().await {
        total += 1;
        println!("Received new peer {:?}, total unique peers {:?}", addr, total);
    }

    println!("search end");
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
