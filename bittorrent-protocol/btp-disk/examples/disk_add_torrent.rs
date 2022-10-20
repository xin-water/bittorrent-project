
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

use std::task::Poll;
use std::pin::Pin;
use btp_disk::{DiskManagerBuilder, IDiskMessage, NativeFileSystem, ODiskMessage};
use btp_metainfo::Metainfo;
use hex;



#[tokio::main]
async fn main() {
    init_log();
    info!("start run .......");

    let torrent_path = "bittorrent-protocol/btp-disk/examples_data/torrent/music.torrent";
    let download_path = "bittorrent-protocol/btp-disk/examples_data/download";

    let mut torrent_bytes = Vec::new();
    File::open(torrent_path)
        .unwrap()
        .read_to_end(&mut torrent_bytes)
        .unwrap();
    let metainfo_file = Metainfo::from_bytes(torrent_bytes).unwrap();

    let native_fs = NativeFileSystem::with_directory(download_path);

    let mut disk_manager = DiskManagerBuilder::new().build(native_fs);

    let (mut disk_send, mut disk_recv) = disk_manager.into_parts();

    let info= metainfo_file.info();
    let info_hash = info.info_hash();
    let total_pieces = info.pieces().count();
    info!("start send msg ");
    disk_send.send(IDiskMessage::AddTorrent(metainfo_file)).await.expect("send message fail");
    disk_send.send(IDiskMessage::CheckTorrent(info_hash)).await.expect("send message fail");

    info!("end send msg ");

    let mut good_pieces = 0;
    let mut bad_pieces = 0;

    while let Some(msg) = disk_recv.recv().await {

         match msg {
             ODiskMessage::FoundGoodPiece(_, _) => {
                good_pieces += 1;
                debug!("{:?}: msg: FoundGoodPiece ", Local::now().naive_local());
             }
             ODiskMessage::FoundBadPiece(_, _) => {
                 bad_pieces += 1;
                 debug!("{:?}: msg: FoundBadPiece ", Local::now().naive_local());
             }
             ODiskMessage::CheckPace(_, pace) => {
                 info!("校验进度{:?}", pace);
             }
             ODiskMessage::DownloadPace(_, pace) => {
                 info!("下载进度{:?}", pace);
             }
             ODiskMessage::TorrentAdded(hash) => {
                info!("Torrent With Hash {:?} Successfully Added", hex::encode(hash));
             }
             ODiskMessage::CheckTorrented(hash) => {
                 info!("Torrent With Hash {:?} Successfully Check Torrent", hex::encode(hash));
                 info!(
                    "Torrent Has {:?} Good Pieces and {:?} Bad Pieces Out Of {:?} Total Pieces",
                     good_pieces,bad_pieces, total_pieces
                 );
                 break;
             }
             unexpected @ _ => panic!("Unexpected ODiskMessage {:?}", unexpected),
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
                .build(LevelFilter::Info),
        )
        .unwrap();

    let _ = log4rs::init_config(config).unwrap();
}