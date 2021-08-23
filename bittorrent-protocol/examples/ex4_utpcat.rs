//! Implementation of a simple uTP client and server.
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

use std::process;
use bittorrent_protocol::utp::UtpStream;
use std::io::{stdin, stdout, stderr, Read, Write};

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

fn usage() -> ! {
    println!("Usage: ex4_utpcat [-s|-c] <address> <port>");
    process::exit(1);
}

fn main() {

    init_log();
    info!("start run .......");

    // This example may run in either server or client mode.
    // Using an enum tends to make the code cleaner and easier to read.
    enum Mode {Server, Client}


    // Fetch arguments
    let mut args = std::env::args();

    // Skip program name
    args.next();

    // Parse the mode argument
    let mode: Mode = match args.next() {
        Some(ref s) if s == "-s" => Mode::Server,
        Some(ref s) if s == "-c" => Mode::Client,
        _ => usage(),
    };

    // Parse the address argument or use a default if none is provided
    let addr = match (args.next(), args.next()) {
        (None, None) => "127.0.0.1:8080".to_owned(),
        (Some(ip), Some(port)) => format!("{}:{}", ip, port),
        _ => usage(),
    };
    let addr: &str = &addr;

    match mode {
        Mode::Server => {
            // Create a listening stream
            let mut stream = UtpStream::bind(addr).expect("Error binding stream");
            let mut writer = stdout();
            let _ = writeln!(&mut stderr(), "Serving on {}", addr);

            // Create a reasonably sized buffer
            let mut payload = vec![0; 1024 * 1024];

            // Wait for a new connection and print the received data to stdout.
            // Reading and printing chunks like this feels more interactive than trying to read
            // everything with `read_to_end` and avoids resizing the buffer multiple times.
            loop {
                match stream.read(&mut payload) {
                    Ok(0) => break,
                    Ok(read) => writer.write(&payload[..read]).expect("Error writing to stdout"),
                    Err(e) => panic!("{}", e)
                };

                match stdin().read(&mut payload) {
                    Ok(0) => {},
                    Ok(read) => {
                        stream.write(&payload[..read]).expect("Error writing to stdout");
                    }
                    Err(e) => panic!("{}", e)
                };
            }
        }
        Mode::Client => {
            // Create a stream and try to connect to the remote address
            let mut stream = UtpStream::connect(addr).expect("Error connecting to remote peer");
            let mut reader = stdin();

            // Create a reasonably sized buffer
            let mut payload = vec![0; 1024 * 1024];

            // Read from stdin and send it to the remote server.
            // Once again, reading and sending small chunks like this avoids having to read the
            // entire input (which may be endless!) before starting to send, unlike what would
            // happen if we were to use `read_to_end` on `reader`.
            loop {
                match reader.read(&mut payload) {
                    Ok(0) => break,
                    Ok(read) => stream.write(&payload[..read]).expect("Error writing to stream"),
                    Err(e) => {
                        stream.close().expect("Error closing stream");
                        panic!("{:?}", e);
                    }
                };

                match stream.read(&mut payload) {
                    Ok(0) => {},
                    Ok(read) =>{
                        stdout().write(&payload[..read]).expect("Error writing to stream");
                    }
                    Err(e) => {
                        stream.close().expect("Error closing stream");
                        panic!("{:?}", e);
                    }
                };
            }

            // Explicitly close the stream.
            stream.close().expect("Error closing stream");
        }
    }
}
