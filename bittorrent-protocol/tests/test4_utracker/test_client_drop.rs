use std::sync::mpsc::{self};

use super::MockHandshaker;
use bittorrent_protocol::util::bt;
use bittorrent_protocol::utracker::announce::{AnnounceEvent, ClientState};
use bittorrent_protocol::utracker::{ClientError, ClientRequest, TrackerClient};

#[test]
#[allow(unused)]
fn positive_client_request_failed() {
    let (send, recv) = mpsc::channel();

    let server_addr = "127.0.0.1:3503".parse().unwrap();
    // Dont actually create the server :) since we want the request to wait for a little bit until we drop

    let mock_handshaker = MockHandshaker::new(send);
    let send_token = {
        let mut client =
            TrackerClient::new("127.0.0.1:4503".parse().unwrap(), mock_handshaker.clone()).unwrap();

        let send_token = client
            .request(
                server_addr,
                ClientRequest::Announce(
                    [0u8; bt::INFO_HASH_LEN].into(),
                    ClientState::new(0, 0, 0, AnnounceEvent::None),
                ),
            )
            .unwrap();

        send_token
    };
    // Client is now dropped

    let metadata = recv.recv().unwrap();

    assert_eq!(send_token, metadata.token());

    match metadata.result() {
        &Err(ClientError::ClientShutdown) => (),
        _ => panic!("Did Not Receive ClientShutdown..."),
    }

    mock_handshaker.connects_received(|connects| {
        assert_eq!(connects.len(), 0);
    });
}
