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
use std::fs::File;
use bittorrent_protocol::utp::{UtpStream,UtpSocket};
use std::io::{Read, Write};

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

pub fn main(){

    // Start logger
    init_log();
    info!("start run .......");

    let mut client = UtpSocket::connect("127.0.0.1:8080").expect("Error binding stream");
    println!("本地端口：{:?}",client.local_addr().unwrap());

    for v in  0..3{

        let _ = client.send_to(b"hello world utp");
        let mut buf = [0;1024];
        let msg_size = client.recv_from(&mut buf);

        let msg = &buf[..msg_size.unwrap().0];
        println!("\n------------------------------------------------");
        println!("接受到消息: {:?}",&msg);
        println!("接受到消息: {:?}",String::from_utf8(msg.to_vec()).unwrap());
    }
}