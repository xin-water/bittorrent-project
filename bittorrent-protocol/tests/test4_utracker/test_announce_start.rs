use std::sync::mpsc::{self};
use std::thread::{self};
use std::time::Duration;

use super::{MockHandshaker, MockTrackerHandler};
use bittorrent_protocol::util::bt;
use bittorrent_protocol::utracker::announce::{AnnounceEvent, ClientState};
use bittorrent_protocol::utracker::{ClientRequest, TrackerClient, TrackerServer};

#[test]
#[allow(unused)]
fn positive_announce_started() {
    let (send, recv) = mpsc::channel();

    let server_addr = "127.0.0.1:3501".parse().unwrap();
    let mock_handler = MockTrackerHandler::new();
    let server = TrackerServer::run(server_addr, mock_handler).unwrap();

    thread::sleep(Duration::from_millis(100));

    let mock_handshaker = MockHandshaker::new(send);
    let mut client =
        TrackerClient::new("127.0.0.1:4501".parse().unwrap(), mock_handshaker.clone()).unwrap();

    let send_token = client
        .request(
            server_addr,
            ClientRequest::Announce(
                [0u8; bt::INFO_HASH_LEN].into(),
                ClientState::new(0, 0, 0, AnnounceEvent::Started),
            ),
        )
        .unwrap();

    let metadata = recv.recv().unwrap();
    assert_eq!(send_token, metadata.token());

    let response = metadata
        .result()
        .as_ref()
        .unwrap()
        .announce_response()
        .unwrap();
    assert_eq!(response.leechers(), 1);
    assert_eq!(response.seeders(), 1);
    assert_eq!(response.peers().iter().count(), 1);

    mock_handshaker.connects_received(|connects| {
        assert_eq!(connects.len(), 1);
    });
}
