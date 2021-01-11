#[macro_use]
extern crate log;

use std::fs::File;
use std::io::{self, BufRead, Read, Write};

use chrono::Local;
use log::{LogLevel, LogLevelFilter, LogMetadata, LogRecord};

use bittorrent_protocol::metainfo::Metainfo;

use bittorrent_protocol::disk::NativeFileSystem;
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage, ODiskMessage};

fn main() {
    log::set_logger(|m| {
        m.set(LogLevelFilter::max());
        Box::new(SimpleLogger)
    }).unwrap();

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

    let download_path = "bittorrent-protocol/examples_data/download";
    let torrent_path = "bittorrent-protocol/examples_data/torrent/node_module.torrent";

    let mut torrent_bytes = Vec::new();
    File::open(torrent_path)
        .unwrap()
        .read_to_end(&mut torrent_bytes)
        .unwrap();
    let metainfo_file = Metainfo::from_bytes(torrent_bytes).unwrap();

    let native_fs = NativeFileSystem::with_directory(download_path);

    let disk_manager = DiskManagerBuilder::new().build(native_fs);

    let (mut disk_send, mut disk_recv) = disk_manager.into_parts();

    let total_pieces = metainfo_file.info().pieces().count();

    println!("{:?}: start send msg ", Local::now().naive_local());
    disk_send.send(IDiskMessage::AddTorrent(metainfo_file));
    println!("{:?}: end send msg ", Local::now().naive_local());

    let mut good_pieces = 0;

    loop {
        let recv_msg = disk_recv.next();
        match recv_msg.unwrap() {
            ODiskMessage::FoundGoodPiece(_, _) => {
                good_pieces += 1;
                println!("{:?}: msg: FoundGoodPiece ", Local::now().naive_local());
            }
            ODiskMessage::TorrentAdded(hash) => {
                println!();
                println!("Torrent With Hash {:?} Successfully Added", hash);
                println!(
                    "Torrent Has {:?} Good Pieces Out Of {:?} Total Pieces",
                    good_pieces, total_pieces
                );
                break;
            }
            unexpected @ _ => panic!("Unexpected ODiskMessage {:?}", unexpected),
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
