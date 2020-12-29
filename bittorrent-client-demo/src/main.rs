#[macro_use]
extern crate hex_literal;

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use std::{any, convert, thread};

use clap::{App, Arg};
use rand::Rng;

mod decoder;
mod hash;
mod meta_info;
mod tracker;
mod tracker_response;

mod request_metadata;
mod request_queue;

mod listener;
mod peer;

use crate::peer::download::{self, Download};
use crate::peer::peer_connection;
use crate::tracker_response::Peer;

const PEER_ID_PREFIX: &'static str = "-btX001-";

fn main() {
    // log::set_logger(|m| {
    //     m.set(LogLevelFilter::max());
    //     Box::new(SimpleLogger)
    // }).unwrap();

    // parse command-line arguments & options
    let matches = App::new("bittorrent")
        .version("0.1.0-alpha")
        .author("xinwater")
        .about("a bittorrent client")
        .arg(
            Arg::with_name("PORT")
                .short("p")
                .long("port")
                .help("set listen port ,default 6881")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("FILE")
                .index(1)
                .help("Sets the bittorrent file to use")
                .required(true),
        )
        .get_matches();

    //设置端口
    let port: u16 = matches.value_of("PORT").unwrap_or("6881").parse().unwrap();

    // 获取文件名
    let filename = matches.value_of("FILE").unwrap();

    //运行
    run(filename, port).unwrap();
}

fn run(filename: &str, listener_port: u16) -> Result<(), Error> {
    println!("Loading {}", filename);
    println!("set listen port {}", listener_port);

    let our_peer_id = generate_peer_id();
    println!("Using peer id: {}", our_peer_id);
    println!("Using peer light: {}", our_peer_id.len());
    // parse .torrent file
    let torrent = meta_info::parse(filename).unwrap();

    // connect to tracker and download list of peers
    let mut peers = Vec::new();
    peers = tracker::get_peers(&our_peer_id, &torrent, listener_port).unwrap();

    // let mut peer = Peer::defalut();
    // peer.ip = Ipv4Addr::new(5, 135, 186, 68);
    // peer.port = 6905;
    // peers.push(peer);

    // let mut peer = Peer::defalut();
    // peer.ip = Ipv4Addr::new(163, 172, 44, 198);
    // peer.port = 62334;
    // peers.push(peer);

    println!("Found {} peers", peers.len());

    // create the download metadata object and stuff it inside a reference-counted mutex
    let downloader = Download::new(our_peer_id.clone(), torrent.clone()).unwrap();
    let downloader_mutex = Arc::new(Mutex::new(downloader));

    let downloader_pieces_spilt = downloader_mutex.clone();

    //单独开启一个线程检验文件，边校验边下载
    let pieces_spilt_thread = thread::spawn(move || {
        download::pieces_spilt(downloader_pieces_spilt);
    });

    // spawn thread to listen for incoming request
    listener::start(listener_port, downloader_mutex.clone());

    // spawn threads to connect to peers and start the download
    let peer_threads: Vec<JoinHandle<()>> = peers
        .into_iter()
        .map(|peer| {
            thread::sleep(Duration::from_secs(2));
            let mutex = downloader_mutex.clone();
            println!("\npeer:{:?}", peer);
            let peer_thread = thread::spawn(move || match peer_connection::connect(&peer, mutex) {
                Ok(_) => println!("Peer done"),
                Err(e) => println!("链接失败: {:?}", e),
            });
            peer_thread
        })
        .collect();

    // loop {
    //     thread::sleep(Duration::from_secs(60));
    //     let (mut v, mut k, (hash_num, mut hase_pieces), close_connect) = {
    //         let mut download;
    //         loop {
    //             if let Ok(mut downloader) = downloader_mutex.lock() {
    //                 download = downloader;
    //                 break;
    //             }
    //         }
    //         (download.peer_channels_num(), download.active_peer_connect(), download.have_pieces(), download.get_close_connext())
    //     };
    //
    //     println!("已关闭连接数:{}", close_connect.len());
    //     for socket_addr in &close_connect {
    //         println!("已关闭连接:{}", socket_addr);
    //     }
    //
    //     println!("活跃线程数{}, 传输线程数:{}", v, k.len());
    //     println!("传输详情:");
    //     for (socket_addr, size) in &k {
    //         println!("peer:{} 传输了 {} MB", socket_addr.ip(), size);
    //     }
    //
    //     let mut bytes: Vec<u8> = vec![0; (hase_pieces.len() as f64 / 8 as f64).ceil() as usize];
    //     for have_index in 0..hase_pieces.len() {
    //         let bytes_index = have_index / 8;
    //         let index_into_byte = have_index % 8;
    //         if hase_pieces[have_index] {
    //             let mask = 1 << (7 - index_into_byte);
    //             bytes[bytes_index] |= mask;
    //         }
    //     };
    //     println!("整体进度:{:?}", bytes);
    //
    //     if v == 0 {
    //         break;
    //     }
    // }

    pieces_spilt_thread.join().unwrap();
    // wait for peers to complete
    for thr in peer_threads {
        thr.join()?;
    }
    Ok(())
}

fn generate_peer_id() -> String {
    let mut rng = rand::thread_rng();
    let mut data = Vec::new();

    for _ in 0..12 {
        let num: u8 = rng.gen_range(0, 3);

        if num == 0 {
            let temp: u8 = rng.gen_range(97, 122);
            data.push(temp);
        } else if num == 1 {
            let temp: u8 = rng.gen_range(65, 90);
            data.push(temp);
        } else {
            let temp: u8 = rng.gen_range(48, 57);
            data.push(temp);
        }
    }
    let mut peer_id_prefix = String::from(PEER_ID_PREFIX);
    let v = String::from_utf8(data).unwrap();
    format!("{}{}", peer_id_prefix, v)
}

#[derive(Debug)]
pub enum Error {
    DecoderError(meta_info::Error),
    // DownloadError(download::Error),
    TrackerError(tracker::TrackerError),
    Any(Box<dyn any::Any + Send>),
}

impl convert::From<meta_info::Error> for Error {
    fn from(err: meta_info::Error) -> Error {
        Error::DecoderError(err)
    }
}
// impl convert::From<download::Error> for Error {
//     fn from(err: download::Error) -> Error {
//         Error::DownloadError(err)
//     }
// }

impl convert::From<tracker::TrackerError> for Error {
    fn from(err: tracker::TrackerError) -> Error {
        Error::TrackerError(err)
    }
}

impl convert::From<Box<dyn any::Any + Send>> for Error {
    fn from(err: Box<dyn any::Any + Send>) -> Error {
        Error::Any(err)
    }
}
