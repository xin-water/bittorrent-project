#[macro_use]
extern crate log;

#[macro_use]
extern crate tokio;

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use chrono::Local;
use log::{debug, error, log_enabled, info};
use simplelog::*;
use futures::{Future, Sink, SinkExt, Stream, StreamExt};
use tokio::runtime::Runtime;
use bittorrent_protocol::metainfo::Metainfo;
use bittorrent_protocol::disk::NativeFileSystem;
use bittorrent_protocol::disk::{DiskManager,DiskManagerBuilder, IDiskMessage, ODiskMessage};
use std::task::Poll;
use std::pin::Pin;

#[tokio::main]
async fn main() {
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Info, Config::default(), TerminalMode::Mixed,ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("my_rust_binary.log").unwrap()),
        ]
    ).unwrap();

    let torrent_path = "bittorrent-protocol/examples_data/torrent/music.torrent";
    let download_path = "bittorrent-protocol/examples_data/download";

    let mut torrent_bytes = Vec::new();
    File::open(torrent_path)
        .unwrap()
        .read_to_end(&mut torrent_bytes)
        .unwrap();
    let metainfo_file = Metainfo::from_bytes(torrent_bytes).unwrap();

    let native_fs = NativeFileSystem::with_directory(download_path);

    let mut disk_manager = DiskManagerBuilder::new().build(native_fs);

    let (mut disk_send, mut disk_recv) = disk_manager.into_parts();

    let total_pieces = metainfo_file.info().pieces().count();

    println!("{:?}: start send msg ", Local::now().naive_local());
    let _= disk_send.send(IDiskMessage::AddTorrent(metainfo_file)).await;
    println!("{:?}: end send msg ", Local::now().naive_local());

    let mut good_pieces = 0;

    while let Some(msg) = disk_recv.next().await {

         match msg {
            ODiskMessage::FoundGoodPiece(_, _) => {
                good_pieces += 1;
                debug!("{:?}: msg: FoundGoodPiece ", Local::now().naive_local());
             }
            ODiskMessage::TorrentAdded(hash) => {
                println!();
                debug!("Torrent With Hash {:?} Successfully Added", hash);
                debug!(
                    "Torrent Has {:?} Good Pieces Out Of {:?} Total Pieces",
                     good_pieces, total_pieces
                 );
                 break;
             }
            unexpected @ _ => panic!("Unexpected ODiskMessage {:?}", unexpected),
         }
    }
}