use bittorrent_protocol::utp::UtpStream;
use std::io::{Read, Write};

pub fn main(){
    let mut stream = UtpStream::connect("127.0.0.1:8080").expect("Error binding stream");
    let _ = stream.write(b"hello world utp");
    let mut buf = [0;1024];
    let msg_size = stream.read(&mut buf);

    let msg = &buf[..msg_size.unwrap()];
    println!("接受到消息: {:?}",&msg);
    println!("接受到消息: {:?}",String::from_utf8(msg.to_vec()).unwrap());
}
