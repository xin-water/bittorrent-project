
#[macro_use]
extern crate tokio;

use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use chrono::Local;
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
use futures::{Future, Sink, SinkExt, Stream, StreamExt};
use tokio::runtime::Runtime;
use bittorrent_protocol::metainfo::Metainfo;
use bittorrent_protocol::disk::NativeFileSystem;
use bittorrent_protocol::disk::{DiskManager,DiskManagerBuilder, IDiskMessage, ODiskMessage};
use std::task::Poll;
use std::pin::Pin;
use hex;

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

#[tokio::main]
async fn main() {
    init_log();
    info!("start run .......");

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

    info!("start send msg ");
    let _= disk_send.send(IDiskMessage::AddTorrent(metainfo_file)).await;
    info!("end send msg ");

    let mut good_pieces = 0;

    while let Some(msg) = disk_recv.next().await {

         match msg {
            ODiskMessage::FoundGoodPiece(_, _) => {
                good_pieces += 1;
                debug!("{:?}: msg: FoundGoodPiece ", Local::now().naive_local());
             }
            ODiskMessage::TorrentAdded(hash) => {
                info!("Torrent With Hash {:?} Successfully Added", hex::encode(hash));
                info!(
                    "Torrent Has {:?} Good Pieces Out Of {:?} Total Pieces",
                     good_pieces, total_pieces
                 );
                 break;
             }
            unexpected @ _ => panic!("Unexpected ODiskMessage {:?}", unexpected),
         }
    }
}