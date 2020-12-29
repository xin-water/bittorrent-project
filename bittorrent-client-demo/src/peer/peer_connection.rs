use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::panic;
use std::sync::mpsc::{channel, Receiver, RecvError, SendError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{any, convert, fmt, io, thread};

use rand::{self, Rng};

use crate::request_queue::RequestQueue;
use crate::tracker_response::Peer;

use super::download;
use super::download::{Download, BLOCK_SIZE};
use super::ipc::IPC;
use super::message::Message;
use crate::peer::util;

const PROTOCOL: &'static str = "BitTorrent protocol";
const MAX_CONCURRENT_REQUESTS: u32 = 10;

pub fn connect(peer: &Peer, download_mutex: Arc<Mutex<Download>>) -> Result<(), Error> {
    PeerConnection::connect(peer, download_mutex)
}

pub fn accept(stream: TcpStream, download_mutex: Arc<Mutex<Download>>) -> Result<(), Error> {
    PeerConnection::accept(stream, download_mutex)
}

pub struct PeerConnection {
    halt: bool,
    download_mutex: Arc<Mutex<Download>>,
    addr: SocketAddr,
    stream: TcpStream,
    me: PeerMetadata,
    them: PeerMetadata,
    incoming_tx: Sender<IPC>,
    outgoing_tx: Sender<Message>,
    upload_in_progress: bool,
    to_request: HashMap<(u32, u32), (u32, u32, u32)>,
    request_size: u8,
    cat_num: usize,
    cat_status: bool,
    cat_num_list: Vec<usize>,
    un_cat_num_list: Vec<usize>,
    download_size: usize,
}

impl PeerConnection {
    fn connect(peer: &Peer, download_mutex: Arc<Mutex<Download>>) -> Result<(), Error> {
        println!("Connecting to  {}:{}", peer.ip, peer.port);
        let stream = TcpStream::connect((peer.ip, peer.port))?;
        PeerConnection::new(stream, download_mutex, true)
    }

    fn accept(stream: TcpStream, download_mutex: Arc<Mutex<Download>>) -> Result<(), Error> {
        println!("Received connection from a peer!");
        PeerConnection::new(stream, download_mutex, false)
    }

    fn new(
        stream: TcpStream,
        download_mutex: Arc<Mutex<Download>>,
        send_handshake_first: bool,
    ) -> Result<(), Error> {
        let mut request;
        loop {
            if let Ok(mut download) = download_mutex.lock() {
                request = download.have_pieces();
                break;
            }
        }

        let (cat_num, have_pieces) = request;
        println!("查询时已分割检验了:{}块", cat_num);
        let num_pieces = have_pieces.len();
        // create & register incoming IPC channel with Download
        let (incoming_tx, incoming_rx) = channel::<IPC>();
        loop {
            if let Ok(mut download) = download_mutex.lock() {
                download.register_peer(incoming_tx.clone());
                break;
            }
        }

        // create outgoing Message channel
        let (outgoing_tx, outgoing_rx) = channel::<Message>();

        let conn = PeerConnection {
            halt: false,
            download_mutex,
            addr: stream.peer_addr().unwrap(),
            stream,
            me: PeerMetadata::new(have_pieces),
            them: PeerMetadata::new(vec![false; num_pieces]),
            incoming_tx,
            outgoing_tx,
            upload_in_progress: false,
            to_request: HashMap::new(),
            request_size: 8,
            cat_num,
            cat_status: false,
            cat_num_list: Vec::new(),
            un_cat_num_list: Vec::new(),
            download_size: 0,
        };

        conn.run(send_handshake_first, incoming_rx, outgoing_rx)?;

        println!("Disconnected");
        Ok(())
    }

    fn run(
        mut self,
        send_handshake_first: bool,
        incoming_rx: Receiver<IPC>,
        outgoing_rx: Receiver<Message>,
    ) -> Result<(), Error> {
        if send_handshake_first {
            self.send_handshake()?;
            self.receive_handshake()?;
        } else {
            self.receive_handshake()?;
            self.send_handshake()?;
        }

        println!("Handshake complete");

        // spawn a thread to funnel incoming messages from the socket into the incoming message channel
        let downstream_funnel_thread = {
            let stream = self.stream.try_clone().unwrap();
            let tx = self.incoming_tx.clone();
            thread::spawn(move || DownstreamMessageFunnel::start(stream, tx))
        };

        // spawn a thread to funnel outgoing messages from the outgoing message channel into the socket
        let upstream_funnel_thread = {
            let stream = self.stream.try_clone().unwrap();
            let tx = self.incoming_tx.clone();
            thread::spawn(move || UpstreamMessageFunnel::start(stream, outgoing_rx, tx))
        };

        // send a bitfield message letting peer know what we have
        self.send_bitfield()?;

        // process messages received on the channel (both from the remote peer, and from Downlad)
        while !self.halt {
            let message = incoming_rx.recv()?;
            self.process(message)?;
        }

        println!("Disconnecting");
        self.stream.shutdown(Shutdown::Both)?;
        downstream_funnel_thread.join()?;
        upstream_funnel_thread.join()?;
        Ok(())
    }

    fn send_handshake(&mut self) -> Result<(), Error> {
        let message = {
            let mut download;
            loop {
                if let Ok(d) = self.download_mutex.lock() {
                    download = d;
                    break;
                }
            }
            let mut message = vec![];
            message.push(PROTOCOL.len() as u8);
            message.extend(PROTOCOL.bytes());
            message.extend(vec![0; 8].into_iter());
            message.extend(download.torrent.info_hash.as_ref().unwrap().iter().cloned());
            message.extend(download.our_peer_id.bytes());
            message
        };
        self.stream.write_all(&message)?;
        Ok(())
    }

    fn receive_handshake(&mut self) -> Result<(), Error> {
        let pstrlen = read_n(&mut self.stream, 1)?;
        read_n(&mut self.stream, pstrlen[0] as u32)?; // ignore pstr
        read_n(&mut self.stream, 8)?; // ignore reserved
        let info_hash = read_n(&mut self.stream, 20)?;
        let peer_id = read_n(&mut self.stream, 20)?;
        loop {
            if let Ok(download) = self.download_mutex.lock() {
                // validate info hash
                if info_hash != *download.torrent.info_hash.as_ref().unwrap() {
                    return Err(Error::InvalidInfoHash);
                }

                // validate peer id
                let our_peer_id: Vec<u8> = download.our_peer_id.bytes().collect();
                if peer_id == our_peer_id {
                    return Err(Error::ConnectingToSelf);
                }
                break;
            }
        }
        Ok(())
    }

    fn send_message(&mut self, message: Message) -> Result<(), Error> {
        //println!("Sending: {:?}", message);
        self.outgoing_tx.send(message)?;
        Ok(())
    }

    fn process(&mut self, ipc: IPC) -> Result<(), Error> {
        println!("process: {:?}", ipc);
        match ipc {
            IPC::Message(message) => self.process_message(message),
            IPC::BlockComplete(piece_index, block_index) => {
                self.to_request.remove(&(piece_index, block_index));
                match self.me.requests.remove(piece_index, block_index) {
                    Some(r) => {
                        self.send_message(Message::Cancel(r.piece_index, r.offset, r.block_length))
                    }
                    None => Ok(()),
                }
            }
            IPC::PieceComplete(piece_index) => {
                self.me.has_pieces[piece_index as usize] = true;
                self.update_my_interested_status()?;
                self.send_message(Message::Have(piece_index))?;
                Ok(())
            }
            IPC::CatComplete => {
                println!("分割完成");
                let mut request;
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        request = download.have_pieces();
                        break;
                    }
                }

                let (num, hase_pieces) = request;

                self.me.has_pieces = hase_pieces;
                self.cat_status = true;
                self.cat_num_list.append(&mut self.un_cat_num_list);

                for i in 0..self.cat_num_list.len() {
                    match self.cat_num_list.pop() {
                        Some(have_index) => {
                            self.queue_blocks(have_index as u32);
                            // println!("process CatComplete: 添加:{}片{}个任务到请求队列", have_index, );
                        }
                        _ => break,
                    }
                }
                self.update_my_interested_status()?;
                self.request_more_blocks()?;
                Ok(())
            }
            IPC::DownloadComplete => {
                self.halt = true;
                self.update_my_interested_status()?;
                Ok(())
            }
            IPC::BlockUploaded => {
                self.upload_in_progress = false;
                self.upload_next_block()?;
                Ok(())
            }
            IPC::Downstream_Error => {
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        download.put_close_connext(self.addr);
                        break;
                    }
                }
                Ok(())
            }
            IPC::Upstream_Error => {
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        download.put_close_connext(self.addr);
                        break;
                    }
                }
                Ok(())
            }
        }
    }

    fn process_message(&mut self, message: Message) -> Result<(), Error> {
        // println!("process_message Received: {:?}", message);
        match message {
            Message::Choke => {
                self.me.is_choked = true;
            }
            Message::Unchoke => {
                if self.me.is_choked {
                    self.me.is_choked = false;
                    self.request_more_blocks()?;
                }
            }
            Message::Interested => {
                self.them.is_interested = true;
                self.unchoke_them()?;
            }
            Message::NotInterested => {
                self.them.is_interested = false;
            }
            Message::Have(have_index) => {
                self.them.has_pieces[have_index as usize] = true;
                if self.cat_status {
                    self.queue_blocks(have_index);
                    self.update_my_interested_status()?;
                    self.request_more_blocks()?;
                }
            }
            Message::Bitfield(bytes) => {
                for have_index in 0..self.them.has_pieces.len() {
                    let bytes_index = have_index / 8;
                    let index_into_byte = have_index % 8;
                    let byte = bytes[bytes_index];
                    let mask = 1 << (7 - index_into_byte);
                    let value = (byte & mask) != 0;
                    self.them.has_pieces[have_index] = value;
                    //TODO
                    // 如果对等点有分片,则查询自己对应的分片是否完整,不完整将其添加到请求队列里.好从对方获取.
                    // 问题:使用了一个线程分,其他线程并发下载时,本地块未切割完,以对方块序号查询时会发生下标跃界.
                    //     例如,刚切割出第400块 ,对方块在401号位,查询就会跃界.
                    //     [解决方法:查询的块大于分割的暂存到缓存里.]
                    if value {
                        if have_index < self.cat_num {
                            self.cat_num_list.push(have_index);
                        } else {
                            self.un_cat_num_list.push(have_index);
                        }
                    }
                }
                //添加self.request_size个块到请求队列,以免阻塞太长时间.
                for i in 0..self.request_size {
                    match self.cat_num_list.pop() {
                        Some(have_index) => {
                            println!("{:?}----------process_message Bitfield:将对方有,而我可能没有的第:{}块中{}个任务添加到请求队列", self.stream.peer_addr(), have_index, self.queue_blocks(have_index as u32));
                        }
                        _ => break,
                    }
                }
                self.update_my_interested_status()?;
                self.request_more_blocks()?;
            }
            Message::Request(piece_index, offset, length) => {
                let block_index = offset / BLOCK_SIZE;
                self.them
                    .requests
                    .add(piece_index, block_index, offset, length);
                self.upload_next_block()?;
            }
            Message::Piece(piece_index, offset, data) => {
                self.download_size += data.len();
                let block_index = offset / BLOCK_SIZE;
                self.me.requests.remove(piece_index, block_index);
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        download.store(
                            self.stream.peer_addr().unwrap(),
                            (self.download_size as f64 / 1024.0 / 1024.0),
                            piece_index,
                            block_index,
                            data,
                        )?;
                        break;
                    }
                }

                //TODO
                // 每完成一块添加一块任务,可能导致任务队列过长,范围过大.不利于集中下载
                // match self.cat_num_list.pop() {
                //     Some(have_index) => {
                //         println!("process_message Piece: 添加:第{}片{}个任务到请求队列", have_index, self.queue_blocks(have_index as u32));
                //     }
                //     _ => {}
                // }
                self.update_my_interested_status()?;
                self.request_more_blocks()?;
            }
            Message::Cancel(piece_index, offset, _) => {
                let block_index = offset / BLOCK_SIZE;
                self.them.requests.remove(piece_index, block_index);
            }
            Message::KeepAlive => {
                println!("{:?}:没有任务,刷新分割列表", self.stream.peer_addr());
                let mut cat_num_list = Vec::new();
                let mut un_cat_num_list = Vec::new();
                let mut send_num = 0;
                let mut index = 0;

                let mut request;
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        request = download.have_pieces();
                        break;
                    }
                }
                let (cat_num, hase_pieces) = request;

                let pieces_num = hase_pieces.len();
                let mut them_hase = self.them.has_pieces.clone();
                them_hase.reverse();

                if cat_num == pieces_num {
                    println!("此时已分割完毕");
                    for i in 0..pieces_num {
                        if them_hase.pop().unwrap() {
                            send_num += self.queue_blocks(i as u32);
                            index = i;
                        }
                    }
                } else {
                    println!("此时未分割完毕");
                    for i in 0..pieces_num {
                        if them_hase.pop().unwrap() {
                            if i < cat_num {
                                cat_num_list.push(i);
                            } else {
                                un_cat_num_list.push(i);
                            }
                        }
                        // println!("i={},pieces_num={},ban_num_list.len={}",i,pieces_num,ban_num_list.len());
                    }

                    for i in 0..self.request_size {
                        match self.cat_num_list.pop() {
                            Some(have_index) => {
                                send_num += self.queue_blocks(have_index as u32);
                                index = have_index;
                                // println!("i={},index={},send_num={}",i,index,send_num);
                            }
                            _ => {
                                println!("cat_num_list is NULL");
                                //TODO 对面可能在请求数据,需判断后再退出
                                // let addr = self.stream.peer_addr().unwrap();
                                // self.stream.shutdown(Shutdown::Both);
                                // panic!("{}线程无任务退出", addr);
                                break;
                            }
                        }
                    }
                }
                self.cat_num = cat_num;
                self.me.has_pieces = hase_pieces;
                self.cat_num_list = cat_num_list;
                self.un_cat_num_list = un_cat_num_list;

                self.update_my_interested_status()?;
                self.request_more_blocks()?;
                println!(
                    "{:?}-----process_message KeepAlive: 添加:第{}块周围{}个任务到请求队列",
                    self.stream.peer_addr(),
                    index,
                    send_num
                );
            }
            _ => return Err(Error::UnknownRequestType(message)),
        };
        Ok(())
    }

    fn queue_blocks(&mut self, piece_index: u32) -> i32 {
        let mut num = 0;
        let mut incomplete_blocks: Vec<(u32, u32)>;
        loop {
            if let Ok(mut download) = self.download_mutex.lock() {
                incomplete_blocks = download.incomplete_blocks_for_piece(piece_index);
                break;
            }
        }

        for (block_index, block_length) in incomplete_blocks {
            if !self.me.requests.has(piece_index, block_index) {
                self.to_request.insert(
                    (piece_index, block_index),
                    (piece_index, block_index, block_length),
                );
                num += 1;
            }
        }
        // println!("查询未完成块后 me.requests :{:?}", self.me.requests);
        // println!("从downloader中查询{}块中未完成的片,添加{}个任务到当前连接的请求队列中",piece_index,num);
        num
    }

    fn update_my_interested_status(&mut self) -> Result<(), Error> {
        let am_interested = self.me.requests.len() > 0 || self.to_request.len() > 0;
        // println!("请求队列中还有:{}个任务,am_interested = {}", self.to_request.len(),am_interested);

        if self.me.is_interested != am_interested {
            self.me.is_interested = am_interested;
            let message = if am_interested {
                Message::Interested
            } else {
                Message::NotInterested
            };
            self.send_message(message)
        } else {
            Ok(())
        }
    }

    fn send_bitfield(&mut self) -> Result<(), Error> {
        let mut bytes: Vec<u8> =
            vec![0; (self.me.has_pieces.len() as f64 / 8 as f64).ceil() as usize];
        for have_index in 0..self.me.has_pieces.len() {
            let bytes_index = have_index / 8;
            let index_into_byte = have_index % 8;
            if self.me.has_pieces[have_index] {
                let mask = 1 << (7 - index_into_byte);
                bytes[bytes_index] |= mask;
            }
        }
        self.send_message(Message::Bitfield(bytes))
    }

    fn request_more_blocks(&mut self) -> Result<(), Error> {
        if self.me.is_choked || !self.me.is_interested || self.to_request.len() == 0 {
            return Ok(());
        }

        while self.me.requests.len() < MAX_CONCURRENT_REQUESTS as usize {
            let len = self.to_request.len();
            if len == 0 {
                return Ok(());
            }

            // remove a block at random from to_request
            let (piece_index, block_index, block_length) = {
                let index = rand::thread_rng().gen_range(0, len);
                let target = self.to_request.keys().nth(index).unwrap().clone();
                self.to_request.remove(&target).unwrap()
            };

            // add a request
            let offset = block_index * BLOCK_SIZE;
            if self
                .me
                .requests
                .add(piece_index, block_index, offset, block_length)
            {
                self.send_message(Message::Request(piece_index, offset, block_length))?;
            }
        }

        Ok(())
    }

    fn unchoke_them(&mut self) -> Result<(), Error> {
        if self.them.is_choked {
            self.them.is_choked = false;
            self.send_message(Message::Unchoke)?;
            self.upload_next_block()?;
        }
        Ok(())
    }

    fn upload_next_block(&mut self) -> Result<(), Error> {
        if self.upload_in_progress || self.them.is_choked || !self.them.is_interested {
            return Ok(());
        }

        match self.them.requests.pop() {
            Some(r) => {
                let mut data;
                loop {
                    if let Ok(mut download) = self.download_mutex.lock() {
                        data = download.retrive_data(&r)?;
                        break;
                    }
                }
                self.upload_in_progress = true;
                self.send_message(Message::Piece(r.piece_index, r.offset, data))
            }
            None => Ok(()),
        }
    }
}

struct PeerMetadata {
    has_pieces: Vec<bool>,
    is_choked: bool,
    is_interested: bool,
    requests: RequestQueue,
}

impl PeerMetadata {
    fn new(has_pieces: Vec<bool>) -> PeerMetadata {
        PeerMetadata {
            has_pieces,
            is_choked: true,
            is_interested: false,
            requests: RequestQueue::new(),
        }
    }
}

struct DownstreamMessageFunnel {
    stream: TcpStream,
    tx: Sender<IPC>,
}

impl DownstreamMessageFunnel {
    fn start(stream: TcpStream, tx: Sender<IPC>) {
        let mut funnel = DownstreamMessageFunnel { stream, tx };
        match funnel.run() {
            Ok(_) => {}
            Err(e) => {
                println!("Downstream_Error: {:?}", e);
                funnel.tx.send(IPC::Downstream_Error).unwrap();
            }
        }
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let message = self.receive_message()?;
            // println!("Downstream_message: {:?}", &message);
            self.tx.send(IPC::Message(message))?;
        }
    }

    fn receive_message(&mut self) -> Result<Message, Error> {
        let message_size = util::bytes_to_u32(&((read_n(&mut self.stream, 4))?));
        if message_size > 0 {
            let message = read_n(&mut self.stream, message_size)?;
            Ok(Message::new(&message[0], &message[1..]))
        } else {
            Ok(Message::KeepAlive)
        }
    }
}

struct UpstreamMessageFunnel {
    stream: TcpStream,
    rx: Receiver<Message>,
    tx: Sender<IPC>,
}

impl UpstreamMessageFunnel {
    fn start(stream: TcpStream, rx: Receiver<Message>, tx: Sender<IPC>) {
        let mut funnel = UpstreamMessageFunnel {
            stream,
            rx,
            tx: tx.clone(),
        };
        match funnel.run() {
            Ok(_) => {}
            Err(e) => {
                println!("Upstream-Error: {:?}", e);
                funnel.tx.send(IPC::Upstream_Error).unwrap();
            }
        }
    }

    fn run(&mut self) -> Result<(), Error> {
        loop {
            let message = self.rx.recv()?;
            println!("Upstream_Message:{:?}", &message);
            let is_block_upload = match message {
                Message::Piece(_, _, _) => true,
                _ => false,
            };

            // do a blocking write to the TCP stream
            self.stream.write_all(&message.serialize())?;

            // notify the main PeerConnection thread that this block is finished
            if is_block_upload {
                self.tx.send(IPC::BlockUploaded)?;
            }
        }
    }
}

fn read_n(stream: &mut TcpStream, bytes_to_read: u32) -> Result<Vec<u8>, Error> {
    let mut buf = vec![];
    read_n_to_buf(stream, &mut buf, bytes_to_read)?;
    Ok(buf)
}
fn read_n_to_buf(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    bytes_to_read: u32,
) -> Result<(), Error> {
    if bytes_to_read == 0 {
        return Ok(());
    }

    let bytes_read = stream.take(bytes_to_read as u64).read_to_end(buf);
    match bytes_read {
        Ok(0) => Err(Error::SocketClosed),
        Ok(n) if n == bytes_to_read as usize => Ok(()),
        Ok(n) => read_n_to_buf(stream, buf, bytes_to_read - n as u32),
        Err(e) => Err(e)?,
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidInfoHash,
    ConnectingToSelf,
    DownloadError(download::Error),
    IoError(io::Error),
    SocketClosed,
    UnknownRequestType(Message),
    ReceiveError(RecvError),
    SendMessageError(SendError<Message>),
    SendIPCError(SendError<IPC>),
    Any(Box<dyn any::Any + Send>),
}

impl convert::From<download::Error> for Error {
    fn from(err: download::Error) -> Error {
        Error::DownloadError(err)
    }
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl convert::From<RecvError> for Error {
    fn from(err: RecvError) -> Error {
        Error::ReceiveError(err)
    }
}

impl convert::From<SendError<Message>> for Error {
    fn from(err: SendError<Message>) -> Error {
        Error::SendMessageError(err)
    }
}

impl convert::From<SendError<IPC>> for Error {
    fn from(err: SendError<IPC>) -> Error {
        Error::SendIPCError(err)
    }
}

impl convert::From<Box<dyn any::Any + Send>> for Error {
    fn from(err: Box<dyn any::Any + Send>) -> Error {
        Error::Any(err)
    }
}
