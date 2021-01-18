use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;

use bittorrent_protocol::handshake::transports::TcpTransport;
use bittorrent_protocol::handshake::{DiscoveryInfo, HandshakerManagerBuilder};
use bittorrent_protocol::util::bt;

#[test]
fn positive_recover_bytes() {

    let mut handshaker_one_addr = "127.0.0.1:0".parse().unwrap();
    let handshaker_one_pid = [4u8; bt::PEER_ID_LEN].into();

    let mut handshaker_one = HandshakerManagerBuilder::new()
        .with_bind_addr(handshaker_one_addr)
        .with_peer_id(handshaker_one_pid)
        .build(TcpTransport)
        .unwrap();

    handshaker_one_addr.set_port(handshaker_one.port());

    thread::spawn(move || {
        let mut stream = TcpStream::connect(handshaker_one_addr).unwrap();
        let mut write_buffer = Vec::new();

        write_buffer.write_all(&[1, 1]).unwrap();
        write_buffer.write_all(&[0u8; 8]).unwrap();
        write_buffer.write_all(&[0u8; bt::INFO_HASH_LEN]).unwrap();
        write_buffer.write_all(&[0u8; bt::PEER_ID_LEN]).unwrap();
        let expect_read_length = write_buffer.len();
        write_buffer.write_all(&[55u8; 100]).unwrap();

        stream.write_all(&write_buffer).unwrap();

        stream
            .read_exact(&mut vec![0u8; expect_read_length][..])
            .unwrap();
    });

    let mut recv_buffer =   vec![0u8; 100];
        handshaker_one
        .poll()
        .map_err(|_| ())
        .and_then(|message| {
            let (_, _, _, _, _, mut sock) = message.into_parts();

            match sock.read(&mut recv_buffer){
                Ok(v)=> Ok(v),
                _ => Ok(0),
            }

        });

    // Assert that our buffer contains the bytes after the handshake
    assert_eq!(vec![55u8; 100], recv_buffer);
}
