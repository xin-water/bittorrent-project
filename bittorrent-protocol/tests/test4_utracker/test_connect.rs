use std::sync::mpsc::{self};
use std::thread::{self};
use std::time::Duration;

use super::{MockHandshaker, MockTrackerHandler};
use bittorrent_protocol::util::bt;
use bittorrent_protocol::utracker::announce::{AnnounceEvent, ClientState};
use bittorrent_protocol::utracker::{ClientRequest, TrackerClient, TrackerServer};

#[test]
#[allow(unused)]
fn positive_receive_connect_id() {
    let (send, recv) = mpsc::channel();

    let server_addr = "127.0.0.1:3505".parse().unwrap();
    let mock_handler = MockTrackerHandler::new();
    let server = TrackerServer::run(server_addr, mock_handler).unwrap();

    thread::sleep(Duration::from_millis(100));

    let mut client =
        TrackerClient::new("127.0.0.1:4505".parse().unwrap(), MockHandshaker::new(send)).unwrap();

    let send_token = client
        .request(
            server_addr,
            ClientRequest::Announce(
                [0u8; bt::INFO_HASH_LEN].into(),
                ClientState::new(0, 0, 0, AnnounceEvent::None),
            ),
        )
        .unwrap();

    let metadata = recv.recv().unwrap();

    assert_eq!(send_token, metadata.token());
    assert!(metadata.result().is_ok());
}
