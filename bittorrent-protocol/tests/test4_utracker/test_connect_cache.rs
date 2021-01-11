use std::sync::mpsc::{self};
use std::thread::{self};
use std::time::Duration;

use super::{MockHandshaker, MockTrackerHandler};
use bittorrent_protocol::util::bt;
use bittorrent_protocol::utracker::{ClientRequest, TrackerClient, TrackerServer};

#[test]
#[allow(unused)]
fn positive_connection_id_cache() {
    let (send, recv) = mpsc::channel();

    let server_addr = "127.0.0.1:3506".parse().unwrap();
    let mock_handler = MockTrackerHandler::new();
    let server = TrackerServer::run(server_addr, mock_handler.clone()).unwrap();

    thread::sleep(Duration::from_millis(100));

    let mock_handshaker = MockHandshaker::new(send);
    let mut client =
        TrackerClient::new("127.0.0.1:4506".parse().unwrap(), mock_handshaker.clone()).unwrap();

    let first_hash = [0u8; bt::INFO_HASH_LEN].into();
    let second_hash = [1u8; bt::INFO_HASH_LEN].into();

    client
        .request(server_addr, ClientRequest::Scrape(first_hash))
        .unwrap();
    recv.recv().unwrap();

    assert_eq!(mock_handler.num_active_connect_ids(), 1);

    for _ in 0..10 {
        client
            .request(server_addr, ClientRequest::Scrape(second_hash))
            .unwrap();
    }

    for _ in 0..10 {
        recv.recv().unwrap();
    }

    assert_eq!(mock_handler.num_active_connect_ids(), 1);
}
