#[macro_use]
extern crate log;

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use chrono::Local;
use log::{debug, error, log_enabled, info};
use simplelog::*;

use bittorrent_protocol::metainfo::Metainfo;
use bittorrent_protocol::disk::NativeFileSystem;
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage, ODiskMessage};
use futures::future::{self, Future, Loop};
use futures::{Sink, Stream};
use tokio::runtime::Runtime;

fn main() {
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Info, Config::default(), TerminalMode::Mixed,ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("my_rust_binary.log").unwrap()),
        ]
    ).unwrap();

    // info!("Utility For Allocating Disk Space For A Torrent File");

    // let stdin = io::stdin();
    // let mut input_lines = stdin.lock().lines();
    // let mut stdout = io::stdout();
    //
    // print!("Enter the destination download directory: ");
    // stdout.flush().unwrap();
    // let download_path = input_lines.next().unwrap().unwrap();
    //
    // print!("Enter the full path to the torrent file: ");
    // stdout.flush().unwrap();
    // let torrent_path = input_lines.next().unwrap().unwrap();

    let torrent_path = "bittorrent-protocol/examples_data/torrent/music.torrent";
    let download_path = "bittorrent-protocol/examples_data/download";

    let mut torrent_bytes = Vec::new();
    File::open(torrent_path)
        .unwrap()
        .read_to_end(&mut torrent_bytes)
        .unwrap();
    let metainfo_file = Metainfo::from_bytes(torrent_bytes).unwrap();

    let native_fs = NativeFileSystem::with_directory(download_path);

    let disk_manager = DiskManagerBuilder::new().build(native_fs);

    let (mut disk_send, mut disk_recv) = disk_manager.split();

    let total_pieces = metainfo_file.info().pieces().count();

    let mut runtime = Runtime::new().unwrap();
    println!("{:?}: start send msg ", Local::now().naive_local());
    runtime.block_on(disk_send.send(IDiskMessage::AddTorrent(metainfo_file)));
    println!("{:?}: end send msg ", Local::now().naive_local());

    let mut good_pieces = 0;
    runtime.block_on(
        future::loop_fn((good_pieces, disk_recv), move |(mut good_pieces, disk_recv)| {
            disk_recv
                .into_future()
                .map(move |(mut recv_msg, mut disk_recv)| {
                    println!("{:?}: msg: {:?}", Local::now().naive_local(), &recv_msg);
                    match recv_msg.unwrap() {
                        ODiskMessage::FoundGoodPiece(_, _) => {
                            good_pieces += 1;
                            Loop::Continue((good_pieces, disk_recv))
                        }
                        ODiskMessage::TorrentAdded(hash) => {
                            println!();
                            println!("Torrent With Hash {:?} Successfully Added", hash);
                            println!(
                                "Torrent Has {:?} Good Pieces Out Of {:?} Total Pieces",
                                good_pieces, total_pieces
                            );
                            Loop::Break(good_pieces)
                        }
                        unexpected @ _ => panic!("Unexpected ODiskMessage {:?}", unexpected),
                    }
                })
        })
            .map_err(|x| {})
            .map(|item| item),
    )
        .unwrap_or_else(|_| panic!("Core Loop Timed Out"));
}