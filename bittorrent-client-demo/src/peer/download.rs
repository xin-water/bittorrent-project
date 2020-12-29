use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::mpsc::{SendError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{convert, io};

use crate::hash::{calculate_sha1, Sha1};

use super::ipc::IPC;
use crate::meta_info::MetaInfo;
use crate::request_metadata::RequestMetadata;

pub const BLOCK_SIZE: u32 = 16384;

pub struct Download {
    pub our_peer_id: String,
    pub torrent: MetaInfo,
    pub pieces: Vec<Piece>,
    file: File,
    cat_complete: bool,
    download_size: usize,
    download_paces: usize,
    peer_channels: Vec<Sender<IPC>>,
    active_connext: HashMap<SocketAddr, f64>,
    close_connext: HashSet<SocketAddr>,
}

pub fn pieces_spilt(downloader_mutex: Arc<Mutex<Download>>) {
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (file_length, piece_length, num_pieces, mut piece_list) = {
        let mut torrent;
        loop {
            if let Ok(mut download) = downloader_mutex.lock() {
                torrent = download.torrent.clone();
                break;
            }
        }
        (
            torrent.info.size.unwrap(),
            torrent.info.piece_length,
            torrent.info.num_pieces.unwrap(),
            torrent.info.piece_list.clone().unwrap(),
        )
    };
    // println!("file_length:{}\npiece_length:{}\nnum_pieces:{}", file_length, piece_length, num_pieces);

    for i in 0..num_pieces {
        let offset = i as u64 * piece_length;
        let length = if i < (num_pieces - 1) {
            piece_length
        } else {
            file_length - offset
        };
        let mut xc: Vec<u8> = piece_list.remove(0);
        let mut piece = Piece::new(length as u32, offset, xc);

        loop {
            if let Ok(mut downloader) = downloader_mutex.lock() {
                piece.verify(&mut downloader.file).unwrap();
                downloader.pieces.push(piece);
                break;
            }
        }
    }
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - start;
    println!("\n分割用时:{}", time);

    loop {
        if let Ok(mut downloader) = downloader_mutex.lock() {
            downloader.cat_complete();
            downloader.broadcast(IPC::CatComplete);
            break;
        }
    }
}

impl Download {
    pub fn new(our_peer_id: String, torrent: MetaInfo) -> Result<Download, Error> {
        let path = Path::new("downloads");
        if !path.exists() {
            fs::create_dir(path).unwrap();
            println!("创建目录");
        }

        let path = Path::new("downloads").join(&torrent.info.name);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)?;

        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // create pieces
        let mut pieces = vec![];
        Ok(Download {
            our_peer_id: our_peer_id,
            torrent: torrent,
            pieces: pieces,
            file: file,
            cat_complete: false,
            download_size: 0,
            download_paces: 0,
            peer_channels: vec![],
            active_connext: HashMap::new(),
            close_connext: HashSet::new(),
        })
    }

    pub fn cat_complete(&mut self) {
        self.cat_complete = true;
    }
    pub fn cat_status(&mut self) -> bool {
        self.cat_complete
    }

    pub fn register_peer(&mut self, channel: Sender<IPC>) {
        self.peer_channels.push(channel);
    }

    pub fn put_close_connext(&mut self, socketaddr: SocketAddr) {
        self.close_connext.insert(socketaddr);
    }

    pub fn get_close_connext(&mut self) -> HashSet<SocketAddr> {
        self.close_connext.clone()
    }

    pub fn peer_channels_num(&mut self) -> usize {
        self.peer_channels.len()
    }

    pub fn active_peer_reset(&mut self) {
        self.active_connext = HashMap::new();
    }
    pub fn active_peer_connect(&mut self) -> HashMap<SocketAddr, f64> {
        self.active_connext.clone()
    }

    pub fn store(
        &mut self,
        socket_addr: SocketAddr,
        download_size: f64,
        piece_index: u32,
        block_index: u32,
        data: Vec<u8>,
    ) -> Result<(), Error> {
        self.active_connext
            .insert(socket_addr.clone(), download_size);
        {
            let piece = &mut self.pieces[piece_index as usize];
            if piece.is_complete || piece.has_block(block_index) {
                // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
                return Ok(());
            }
            let len = data.len();
            piece.store(&mut self.file, block_index, data)?;
            self.download_size += len;
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index));

        // notify peers if piece is complete
        if self.pieces[piece_index as usize].is_complete {
            self.broadcast(IPC::PieceComplete(piece_index));
            self.download_paces += 1;
            println!(
                "已下载{}块 - {} MB",
                self.download_paces,
                (self.download_size as f64) / 1024.0 / 1024.0
            );
        }

        // notify peers if download is complete
        if self.is_complete() {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete);
        }
        Ok(())
    }

    pub fn retrive_data(&mut self, request: &RequestMetadata) -> Result<Vec<u8>, Error> {
        let ref piece = self.pieces[request.piece_index as usize];
        if piece.is_complete {
            let offset = piece.offset + request.offset as u64;
            let file = &mut self.file;
            file.seek(io::SeekFrom::Start(offset))?;
            let mut buf = vec![];
            file.take(request.block_length as u64)
                .read_to_end(&mut buf)?;
            Ok(buf)
        } else {
            Err(Error::MissingPieceData)
        }
    }

    pub fn have_pieces(&self) -> (usize, Vec<bool>) {
        let mut have_pieces: Vec<bool> = self.pieces.iter().map(|p| p.is_complete).collect();
        let cat_num = have_pieces.len();
        let num_pieces = self.torrent.info.num_pieces.unwrap();

        //采用边分割,边下载时,块可能还未分割完,需要补全空位.
        if cat_num < num_pieces as usize {
            let mut no_size: Vec<bool> = vec![false; num_pieces as usize - cat_num];
            have_pieces.append(&mut no_size);
        };

        (cat_num, have_pieces)
    }

    //查询{piece_index}片中块的完成情况,返回未完成的片
    pub fn incomplete_blocks_for_piece(&self, piece_index: u32) -> Vec<(u32, u32)> {
        let ref piece = self.pieces[piece_index as usize];
        if !piece.is_complete {
            piece
                .blocks
                .iter()
                .filter(|b| !b.is_complete)
                .map(|b| (b.index, b.length))
                .collect()
        } else {
            vec![]
        }
    }

    fn is_complete(&self) -> bool {
        for piece in self.pieces.iter() {
            if !piece.is_complete {
                return false;
            }
        }
        true
    }

    pub fn broadcast(&mut self, ipc: IPC) {
        self.peer_channels.retain(|channel| {
            match channel.send(ipc.clone()) {
                Ok(_) => true,
                Err(SendError(_)) => false, // presumably channel has disconnected
            }
        });
    }
}

#[derive(Debug)]
pub struct Piece {
    length: u32,
    offset: u64,
    hash: Sha1,
    blocks: Vec<Block>,
    is_complete: bool,
}

impl Piece {
    fn new(length: u32, offset: u64, hash: Sha1) -> Piece {
        // create blocks
        let mut blocks = vec![];
        let num_blocks = (length as f64 / BLOCK_SIZE as f64).ceil() as u32;
        for i in 0..num_blocks {
            let len = if i < (num_blocks - 1) {
                BLOCK_SIZE
            } else {
                length - (BLOCK_SIZE * (num_blocks - 1))
            };
            blocks.push(Block::new(i, len));
        }

        Piece {
            length: length,
            offset: offset,
            hash: hash,
            blocks: blocks,
            is_complete: false,
        }
    }

    fn store(&mut self, file: &mut File, block_index: u32, data: Vec<u8>) -> Result<(), Error> {
        {
            // store data in the appropriate point in the file
            let offset = self.offset + (block_index * BLOCK_SIZE) as u64;
            file.seek(io::SeekFrom::Start(offset))?;
            file.write_all(&data)?;
            self.blocks[block_index as usize].is_complete = true;
        }

        if self.has_all_blocks() {
            let valid = self.verify(file)?;
            if !valid {
                self.reset_blocks();
            }
        }

        Ok(())
    }

    //TODO
    fn verify(&mut self, file: &mut File) -> Result<bool, Error> {
        // read in the part of the file corresponding to the piece
        file.seek(io::SeekFrom::Start(self.offset))?;
        let mut data = vec![];
        file.take(self.length as u64).read_to_end(&mut data)?;

        // calculate the hash, verify it, and update is_complete
        self.is_complete = self.hash == calculate_sha1(&data);
        Ok(self.is_complete)
    }

    fn has_block(&self, block_index: u32) -> bool {
        self.blocks[block_index as usize].is_complete
    }

    fn has_all_blocks(&self) -> bool {
        for block in self.blocks.iter() {
            if !block.is_complete {
                return false;
            }
        }
        true
    }

    fn reset_blocks(&mut self) {
        for block in self.blocks.iter_mut() {
            block.is_complete = false;
        }
    }
}

#[derive(Debug)]
pub struct Block {
    index: u32,
    length: u32,
    is_complete: bool,
}

impl Block {
    fn new(index: u32, length: u32) -> Block {
        Block {
            index: index,
            length: length,
            is_complete: false,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    MissingPieceData,
    IoError(io::Error),
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}
