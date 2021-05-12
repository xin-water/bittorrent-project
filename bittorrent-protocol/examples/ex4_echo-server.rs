#[macro_use]
extern crate log;

use simplelog::*;
use bittorrent_protocol::utp::{UtpListener, UtpSocket};
use std::thread;
use std::fs::File;

fn handle_client(mut s: UtpSocket) {
    let mut buf = [0; 1500];

    loop {
        // Reply to a data packet with its own payload, then end the connection
        match s.recv_from(&mut buf) {

            Ok((0, src)) => {
                info!("<= [{}] disconnect", src);
                break;
            }
            Ok((nread, src)) => {
                info!("<= [{}] {:?}", src, &buf[..nread]);
                let _ = s.send_to(&buf[..nread]);
            }
            Err(e) => println!("{}", e)
        }
    }
}

fn main() {
    // Start logger
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Info, Config::default(), TerminalMode::Mixed,ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("my_rust_binary.log").unwrap()),
        ]
    ).unwrap();

    // Create a listener
    let addr = "127.0.0.1:8080";
    let listener = UtpListener::bind(addr).expect("Error binding listener");

    for connection in listener.incoming() {
        // Spawn a new handler for each new connection
        match connection {
            Ok((socket, _src)) => { thread::spawn(move || { handle_client(socket) }); },
            _ => ()
        }
    }
}
