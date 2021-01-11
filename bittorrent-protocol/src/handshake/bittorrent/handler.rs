 use std::collections::HashSet;
use std::io::{self, Cursor, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread::{self};

use crate::util::bt;
use crate::util::bt::{InfoHash, PeerId};
use nom::{be_u8, IResult};

use crate::handshake::bittorrent::base::{BTPeer, Task};
use std::sync::mpsc::Sender;

 /*
  BitTorrent
    Protocol Name Length: 19
    Protocol Name: BitTorrent protocol
    Reserved Extension Bytes: 0000000000100005
    SHA1 Hash of info dictionary: 3c4fffbc472671e16a6cf7b6473ba9a8bf1b1a3a
    Peer ID: 2d5452333030302d336268777537683035623239
 */

 //拓展常量定义

 //需要8个字节
 const RESERVED_BYTES_LEN: usize = 8;
 //协商拓展在第五位，十六进制值为 10, 十进制值为16
 const EXTENDED_EXT: u8 = 16;
 //dht端口拓展，在最后1位,值为1
 const PORT_EXT: u8 = 1;

 pub fn spawn_handshaker<T>(
    chan: Sender<T>,
    listen: SocketAddr,
    pid: PeerId,
    protocol: &'static str,
    interest: Arc<RwLock<HashSet<InfoHash>>>,
) -> io::Result<(Sender<Task<T>>, u16)>
where
    T: Send + 'static + From<BTPeer>,

{
    let (tx,rx)= std::sync::mpsc::channel::<Task<T>>();
    let chan_client = chan.clone();
    let interest_client = interest.clone();
    let is_exit = Arc::new(RwLock::new(false));
    let is_exit_client =  is_exit.clone();
    thread::spawn(move || {
        loop {
            let msg = rx.recv().unwrap();
            let chan= chan_client.clone();
            let interest =interest_client.clone();
            match msg{
                Task::Connect(opt_pid, hash, addr) => {
                    thread::spawn( move ||{
                        if !interest.read().unwrap().contains(&hash) {
                            return;
                        }
                        let out_buffer = premade_out_buffer(protocol, [0u8; bt::INFO_HASH_LEN].into(), pid);

                        let stream= TcpStream::connect(&addr).ok().unwrap();

                        let mut peer_connection = Connection::new_client_connect(
                            stream,
                            out_buffer,
                            protocol,
                            hash,
                            opt_pid,
                        );
                        match  peer_connection.write(){
                            ConnectionState::RegisterRead => {
                                match peer_connection.read(|hash| interest.read().unwrap().contains(&hash)){
                                    ConnectionState::Completed => {
                                        let (tcp, hash, pid) = peer_connection.destory();
                                        let tcp_peer = BTPeer::new(tcp, hash, pid);
                                        chan.send(tcp_peer.into()).unwrap();
                                    }
                                    _ => { panic!("bittorrent-protocol_handshake: Failed To Client Connectio Read"); }
                                }
                            }
                            _ => { panic!("bittorrent-protocol_handshake: Failed To Client Connectio Write"); }
                        }
                    });
                }
                Task::Metadata(metadata) => {
                    chan.send(metadata).unwrap();
                }
                Task::Shutdown => {
                    let mut v = is_exit_client.write().unwrap();
                    *v = true;
                    panic!("bittorrent-protocol_handshake: Exit To Client");
                }
            }
        }
    });

    let tcp_listener = TcpListener::bind(&listen)?;
    let listen_port = tcp_listener.local_addr()?.port();
    thread::spawn(move || {
        loop {
            let (stream,_s) = tcp_listener.accept().unwrap();
            let chan= chan.clone();
            let interest = interest.clone();
            thread::spawn(move ||{
                let out_buffer = premade_out_buffer(protocol, [0u8; bt::INFO_HASH_LEN].into(), pid);
                let mut peer_connection =  Connection::new_server_connect(
                    stream,
                    out_buffer,
                    protocol
                );
                match peer_connection.read(|hash| interest.read().unwrap().contains(&hash)){
                    ConnectionState::RegisterWrite => {
                        match peer_connection.write(){
                            ConnectionState::Completed  =>{
                                let (tcp, hash, pid) = peer_connection.destory();
                                let tcp_peer = BTPeer::new(tcp, hash, pid);

                                chan.send(tcp_peer.into()).unwrap();
                            }
                            _ => { panic!("bittorrent-protocol_handshake: Failed To Server Connectio Write"); }
                        }
                    }
                    _ => { panic!("bittorrent-protocol_handshake: Failed To Server Connectio Read"); }
                }
            });

            if *is_exit.read().unwrap() {
                panic!("bittorrent-protocol_handshake: Exit To Server");
            }
        }
    });

    Ok((tx, listen_port))
}


//----------------------------------------------------------------------------//

fn premade_out_buffer(protocol: &'static str, info_hash: InfoHash, pid: PeerId) -> Vec<u8> {
    let buffer_len = 1 + protocol.len() + RESERVED_BYTES_LEN + bt::INFO_HASH_LEN + bt::PEER_ID_LEN;
    let mut buffer = Vec::with_capacity(buffer_len);

    buffer.write(&[protocol.len() as u8]).unwrap();
    buffer.write(protocol.as_bytes()).unwrap();

    buffer.write(&[0u8; RESERVED_BYTES_LEN - 3]).unwrap();
    buffer.write(&[EXTENDED_EXT]).unwrap();
    buffer.write(&[0u8]).unwrap();
    buffer.write(&[PORT_EXT]).unwrap();

    buffer.write(info_hash.as_ref()).unwrap();
    buffer.write(pid.as_ref()).unwrap();

    buffer
}

//----------------------------------------------------------------------------//

enum ConnectionState {
    RegisterRead,
    RegisterWrite,
    Disconnect,
    Completed,
}

struct Connection {
    out_buffer: Cursor<Vec<u8>>,
    in_buffer: Cursor<Vec<u8>>,
    remote_stream: TcpStream,
    opt_pid: Option<PeerId>,
    info_hash: InfoHash,
    bt_protocol: &'static str,
    // Whether or not we have flipped read/write states yet
    // so we know when we can validate the complete handshake
    is_client: bool,
}

impl Connection {

    pub fn new_client_connect(
        stream: TcpStream,
        out_buffer: Vec<u8>,
        bt_protocol: &'static str,
        info_hash: InfoHash,
        opt_pid: Option<PeerId>,
    ) -> Connection {
        Connection::new_initiate(stream, out_buffer, bt_protocol, info_hash, opt_pid, true)
    }

    pub fn new_server_connect(
        stream: TcpStream,
        out_buffer: Vec<u8>,
        bt_protocol: &'static str,
    ) -> Connection {
        let dummy_hash = [0u8; bt::INFO_HASH_LEN].into();

        Connection::new_initiate(stream, out_buffer, bt_protocol, dummy_hash, None, false)
    }

    pub fn new_initiate(
        stream: TcpStream,
        mut out_buffer: Vec<u8>,
        bt_protocol: &'static str,
        info_hash: InfoHash,
        opt_pid: Option<PeerId>,
        is_client:bool
    ) -> Connection {
        let in_buffer = Cursor::new(vec![0u8; out_buffer.len()]);

        rewrite_out_hash(&mut out_buffer[..], bt_protocol.len(), info_hash);

        Connection {
            out_buffer: Cursor::new(out_buffer),
            in_buffer: in_buffer,
            remote_stream: stream,
            opt_pid: opt_pid,
            info_hash: info_hash,
            bt_protocol: bt_protocol,
            is_client: is_client,
        }
    }

    pub fn destory(self) -> (TcpStream, InfoHash, PeerId) {
        (
            self.remote_stream,
            self.info_hash,
            self.opt_pid.unwrap(),
        )
    }


    pub fn get_remote_stream(&self) -> &TcpStream {
        &self.remote_stream
    }

    pub fn read<I>(&mut self, interested_hash: I) -> ConnectionState
    where
        I: Fn(InfoHash) -> bool,
    {
        let total_buf_size = self.in_buffer.get_ref().len();
        let mut read_position = self.in_buffer.position() as usize;

        // Read until we receive an error, we filled our buffer, or we read zero bytes
        let mut read_result = Ok(1);
        while read_result.is_ok()
            && *read_result.as_ref().unwrap() != 0
            && read_position != total_buf_size
        {
            let in_slice = &mut self.in_buffer.get_mut()[read_position..];

            read_result = self.remote_stream.read(in_slice);
            if let Ok(bytes_read) = read_result {
                read_position += bytes_read;
            }
        }
        self.in_buffer.set_position(read_position as u64);

        // Try to parse whatever part of the message we currently have (see if we need to disconnect early)
        let parse_status = {
            let in_slice = &self.in_buffer.get_mut()[..read_position];
            parse_remote_handshake(in_slice, self.opt_pid, self.bt_protocol)
        };
        // If we are flipping over to writing, that means we read first in which case we need to validate
        // the hash against the passed closure, otherwise just use the expected hash since that has already
        // been validated...and its cheaper!!!
        match (parse_status, read_result) {
            (ParseStatus::Valid(hash, pid), _) if self.is_client => {
                self.opt_pid = Some(pid);

                if self.info_hash == hash {
                    ConnectionState::Completed
                } else {
                    ConnectionState::Disconnect
                }
            }
            (ParseStatus::Valid(hash, pid), _) => {
                self.opt_pid = Some(pid);
                self.info_hash = hash;

                if interested_hash(hash) {
                    rewrite_out_hash(
                        self.out_buffer.get_mut(),
                        self.bt_protocol.len(),
                        self.info_hash,
                    );

                    ConnectionState::RegisterWrite
                } else {
                    ConnectionState::Disconnect
                }
            }
            (ParseStatus::Error, _) => ConnectionState::Disconnect,
            (ParseStatus::More, Ok(_)) => ConnectionState::Disconnect,
            (ParseStatus::More, Err(error)) => {
                // If we received an interrupt, we can try again, if we received a would block, need to wait again, otherwise disconnect
                match error.kind() {
                    ErrorKind::Interrupted => self.read(interested_hash),
                    _ => ConnectionState::Disconnect,
                }
            }
        }
    }

    pub fn write(&mut self) -> ConnectionState {
        let total_buf_size = self.out_buffer.get_ref().len();
        let mut write_position = self.out_buffer.position() as usize;

        // Write until we receive an error, we wrote out buffer, or we wrote zero bytes
        let mut write_result = Ok(1);
        while write_result.is_ok()
            && *write_result.as_ref().unwrap() != 0
            && write_position != total_buf_size
        {
            let out_slice = &self.out_buffer.get_ref()[write_position..];

            write_result = self.remote_stream.write(out_slice);
            if let Ok(bytes_wrote) = write_result {
                write_position += bytes_wrote;
            }
        }
        self.out_buffer.set_position(write_position as u64);

        // If we didnt write whole buffer but received an Ok (where wrote == 0), then we assume the peer disconnected
        match (write_result, write_position == total_buf_size) {
            (_, true) if self.is_client => ConnectionState::RegisterRead,
            (_, true) => {
                ConnectionState::Completed
            }
            (Ok(_), false) => ConnectionState::Disconnect,
            (Err(error), false) => match error.kind() {
                ErrorKind::Interrupted => self.write(),
                _ => ConnectionState::Disconnect,
            },
        }
    }
}

fn rewrite_out_hash(buffer: &mut [u8], prot_len: usize, hash: InfoHash) {
    let hash_offset = 1 + prot_len + RESERVED_BYTES_LEN;

    for (dst, src) in buffer[hash_offset..].iter_mut().zip(hash.as_ref().iter()) {
        *dst = *src;
    }
}

enum ParseStatus {
    Valid(InfoHash, PeerId),
    Error,
    More,
}

/// Returns Some(true) if the remote handshake is valid, Some(false) if the remote handshake is invalid, or None if more bytes need to be read.
fn parse_remote_handshake(
    bytes: &[u8],
    expected_pid: Option<PeerId>,
    expected_protocol: &'static str,
) -> ParseStatus {
    let parse_result = do_parse!(
        bytes,
        _unused_prot: call!(parse_remote_protocol, expected_protocol) >>
        _unused_ext: take!(RESERVED_BYTES_LEN) >>
        hash: call!(parse_remote_hash) >>
        pid: call!(parse_remote_pid, expected_pid) >>
        (hash, pid)
    );

    match parse_result {
        IResult::Done(_, (hash, pid)) => ParseStatus::Valid(hash, pid),
        IResult::Error(_) => ParseStatus::Error,
        IResult::Incomplete(_) => ParseStatus::More,
    }
}

fn parse_remote_protocol<'a>(
    bytes: &'a [u8],
    expected_protocol: &'static str,
) -> IResult<&'a [u8], &'a [u8]> {
    let expected_length = expected_protocol.len() as u8;

    switch!(bytes, map!(be_u8, |len| len == expected_length),
        true => tag!(expected_protocol.as_bytes())
    )
}

fn parse_remote_hash(bytes: &[u8]) -> IResult<&[u8], InfoHash> {
    map!(
         bytes,
         take!(bt::INFO_HASH_LEN),
         |hash| InfoHash::from_hash(hash ).unwrap()
    )
}

fn parse_remote_pid(bytes: &[u8], opt_expected_pid: Option<PeerId>) -> IResult<&[u8], PeerId> {
    if let Some(expected_pid) = opt_expected_pid {
        map!(
             bytes,
             tag!(expected_pid.as_ref()),
             |id| PeerId::from_hash(id).unwrap()
        )
    } else {
        map!(
             bytes,
             take!(bt::PEER_ID_LEN),
             |id| PeerId::from_hash(id).unwrap()
        )
    }
}
