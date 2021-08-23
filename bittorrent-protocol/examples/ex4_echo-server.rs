
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
use bittorrent_protocol::utp::{UtpListener, UtpSocket};
use std::thread;
use std::fs::File;

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
    init_log();
    info!("start run .......");

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
