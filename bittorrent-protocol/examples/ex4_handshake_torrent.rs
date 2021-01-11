use std::net::{SocketAddr, ToSocketAddrs};
use std::thread;
use std::time::Duration;
use hex;
use std::sync::mpsc::{self, Receiver, Sender};

use bittorrent_protocol::handshake::{BTHandshaker, BTPeer, Handshaker};

fn main() {
    let (send, recv): (Sender<BTPeer>, Receiver<BTPeer>) = mpsc::channel();
    let peer_id = (*b"-UT2060-000000000000").into();
    let mut handshaker = BTHandshaker::new(send, "127.0.0.1:0".parse().unwrap(), peer_id).unwrap();

    // 从终端读取 种子 hash
    // let mut stdout = io::stdout();
    // let stdin = io::stdin();
    // let mut lines = stdin.lock().lines();
    //
    // stdout.write(b"Enter An InfoHash In Hex Format: ").unwrap();
    // stdout.flush().unwrap();
    //
    // let hex_hash = lines.next().unwrap().unwrap();

    // 使用默认 种子 hash
    let bt_hash_hex = "3c4fffbc472671e16a6cf7b6473ba9a8bf1b1a3a";

    let hash = hex_to_bytes(&bt_hash_hex).into();
    handshaker.register_hash(hash);

    // 从终端读取 peer 地址
    // stdout.write(b"Enter An Address And Port (eg: addr:port): ").unwrap();
    // stdout.flush().unwrap();
    // let str_addr = lines.next().unwrap().unwrap();

    // 从使用默认 peer 地址
    let str_addr = "127.0.0.1:44444";

    // 连接自己
    // let str_addr = format!("127.0.0.1:{:?}", handshaker.port());

    let addr = str_to_addr(&str_addr);
    handshaker.connect(None, hash, addr);
    println!("\nConnection With Peer ");
    let btpeer=recv.recv().unwrap().destory();
    println!("info hash:{:?},\n\
              peer id :{:?}",
              hex::encode(btpeer.1),
              String::from_utf8_lossy(btpeer.2.as_ref())
    );

    handshaker.drop();
    thread::sleep(Duration::from_secs(10));
}

fn hex_to_bytes(hex: &str) -> [u8; 20] {
    let mut exact_bytes = [0u8; 20];

    for byte_index in 0..20 {
        let high_index = byte_index * 2;
        let low_index = (byte_index * 2) + 1;

        let hex_chunk = &hex[high_index..low_index + 1];
        let byte_value = u8::from_str_radix(hex_chunk, 16).unwrap();

        exact_bytes[byte_index] = byte_value;
    }

    exact_bytes
}

fn str_to_addr(addr: &str) -> SocketAddr {
    addr.to_socket_addrs().unwrap().next().unwrap()
}
