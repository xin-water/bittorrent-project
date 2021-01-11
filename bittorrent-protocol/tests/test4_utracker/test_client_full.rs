use std::mem::{self};
use std::sync::mpsc::{self};

use super::MockHandshaker;
use bittorrent_protocol::util::bt;
use bittorrent_protocol::utracker::announce::{AnnounceEvent, ClientState};
use bittorrent_protocol::utracker::{ClientRequest, TrackerClient};

#[test]
#[allow(unused)]
fn positive_client_request_dropped() {
    let (send, recv) = mpsc::channel();

    let server_addr = "127.0.0.1:3504".parse().unwrap();

    let request_capacity = 10;

    let mock_handshaker = MockHandshaker::new(send);
    let mut client = TrackerClient::with_capacity(
        "127.0.0.1:4504".parse().unwrap(),
        mock_handshaker.clone(),
        request_capacity,
    )
    .unwrap();

    for _ in 0..request_capacity {
        client
            .request(
                server_addr,
                ClientRequest::Announce(
                    [0u8; bt::INFO_HASH_LEN].into(),
                    ClientState::new(0, 0, 0, AnnounceEvent::Started),
                ),
            )
            .unwrap();
    }

    assert!(client
        .request(
            server_addr,
            ClientRequest::Announce(
                [0u8; bt::INFO_HASH_LEN].into(),
                ClientState::new(0, 0, 0, AnnounceEvent::Started)
            )
        )
        .is_none());

    mock_handshaker.connects_received(|connects| {
        assert_eq!(connects.len(), 0);
    });

    mem::drop(client);

    for _ in 0..request_capacity {
        recv.recv();
    }
}
