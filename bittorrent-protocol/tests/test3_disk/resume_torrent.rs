use futures::{SinkExt, StreamExt};
use tokio::test;
use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage, ODiskMessage};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};

#[test]
async fn positive_complete_torrent() {
    // Create some "files" as random bytes
    let data_a = (super::random_buffer(1023), "/path/to/file/a".into());
    let data_b = (super::random_buffer(2000), "/path/to/file/b".into());

    // Create our accessor for our in memory files and create a torrent file for them
    let files_accessor = MultiFileDirectAccessor::new(
        "/my/downloads/".into(),
        vec![data_a.clone(), data_b.clone()],
    );
    let metainfo_bytes = MetainfoBuilder::new()
        .set_piece_length(PieceLength::Custom(1024))
        .build(1, files_accessor, |_| ())
        .unwrap();
    let metainfo_file = Metainfo::from_bytes(metainfo_bytes).unwrap();
    let info_hash = metainfo_file.info().info_hash();

    // Spin up a disk manager and add our created torrent to it
    let filesystem = InMemoryFileSystem::new();
    let disk_manager = DiskManagerBuilder::new().build(filesystem.clone());

    let (mut blocking_send, mut recv) = disk_manager.into_parts();

    blocking_send
        .send(IDiskMessage::AddTorrent(metainfo_file.clone()))
        .unwrap();

    // Run a core loop until we get the TorrentAdded message
    let mut good_pieces = 0;
    loop {
        let msg = recv.next().unwrap();
        match msg {
            ODiskMessage::TorrentAdded(_) => break,
            ODiskMessage::FoundGoodPiece(_, _) => good_pieces += 1,
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }
    // Make sure we have no good pieces
    assert_eq!(0, good_pieces);

    // Send a couple blocks that are known to be good, then one bad block
    let mut files_bytes = Vec::new();
    files_bytes.extend_from_slice(&data_a.0);
    files_bytes.extend_from_slice(&data_b.0);

    // Send piece 0
    super::send_block(
        blocking_send.clone(),
        &files_bytes[0..500],
        metainfo_file.info().info_hash(),
        0,
        0,
        500,
        |_| (),
    );
    super::send_block(
        blocking_send.clone(),
        &files_bytes[500..1000],
        metainfo_file.info().info_hash(),
        0,
        500,
        500,
        |_| (),
    );
    super::send_block(
        blocking_send.clone(),
        &files_bytes[1000..1024],
        metainfo_file.info().info_hash(),
        0,
        1000,
        24,
        |_| (),
    );

    // Verify that piece 0 is good
    let mut piece_zero_good = false;
    let mut messages_recvd = 0;

    loop {
        let msg = recv.next().unwrap();
        let messages_recvd = messages_recvd + 1;

        // Map BlockProcessed to a None piece index so we don't update our state
        let (opt_piece_index, new_value) = match msg {
            ODiskMessage::FoundGoodPiece(_, index) => (Some(index), true),
            ODiskMessage::FoundBadPiece(_, index) => (Some(index), false),
            ODiskMessage::BlockProcessed(_) => (None, false),
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };

        match opt_piece_index {
            None => piece_zero_good = false,
            Some(0) => piece_zero_good = new_value,
            Some(x) => panic!("Unexpected Index {:?}", x),
        };

        if messages_recvd == (3 + 1) {
            break;
        }
    }
    // Assert whether or not pieces were good
    assert_eq!(true, piece_zero_good);

    // Remove the torrent from our manager
    blocking_send
        .send(IDiskMessage::RemoveTorrent(info_hash))
        .unwrap();

    // Verify that our torrent was removed
    loop {
        let msg = recv.next().unwrap();
        match msg {
            ODiskMessage::TorrentRemoved(_) => break,
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }

    // Re-add our torrent and verify that we see our good first block
    blocking_send
        .send(IDiskMessage::AddTorrent(metainfo_file.clone()))
        .unwrap();

    let mut piece_zero_good = false;
    loop {
        let msg = recv.next().unwrap();
        match msg {
            ODiskMessage::TorrentAdded(_) => break,
            ODiskMessage::FoundGoodPiece(_, piece) if piece == 0 => {
                piece_zero_good = true;
            }
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }

    assert_eq!(true, piece_zero_good);

    // Send piece 1
    super::send_block(
        blocking_send.clone(),
        &files_bytes[(1024 + 0)..(1024 + 500)],
        metainfo_file.info().info_hash(),
        1,
        0,
        500,
        |_| (),
    );
    super::send_block(
        blocking_send.clone(),
        &files_bytes[(1024 + 500)..(1024 + 1000)],
        metainfo_file.info().info_hash(),
        1,
        500,
        500,
        |_| (),
    );
    super::send_block(
        blocking_send.clone(),
        &files_bytes[(1024 + 1000)..(1024 + 1024)],
        metainfo_file.info().info_hash(),
        1,
        1000,
        24,
        |_| (),
    );

    // Send piece 2
    super::send_block(
        blocking_send.clone(),
        &files_bytes[(2048 + 0)..(2048 + 500)],
        metainfo_file.info().info_hash(),
        2,
        0,
        500,
        |_| (),
    );
    super::send_block(
        blocking_send.clone(),
        &files_bytes[(2048 + 500)..(2048 + 975)],
        metainfo_file.info().info_hash(),
        2,
        500,
        475,
        |_| (),
    );

    // Verify last two blocks are good
    let mut piece_one_good = false;
    let mut piece_two_good = false;

    loop {
        let msg = recv.next().unwrap();
        let messages_recvd = messages_recvd + 1;
        // Map BlockProcessed to a None piece index so we don't update our state
        let (opt_piece_index, new_value) = match msg {
            ODiskMessage::FoundGoodPiece(_, index) => (Some(index), true),
            ODiskMessage::FoundBadPiece(_, index) => (Some(index), false),
            ODiskMessage::BlockProcessed(_) => (None, false),
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };

        match opt_piece_index {
            None => {}
            Some(1) => piece_one_good = new_value,
            Some(2) => piece_two_good = new_value,
            Some(x) => panic!("Unexpected Index {:?}", x),
        };

        if messages_recvd == (5 + 2) {
            break;
        }
    }
    assert_eq!(true, piece_one_good);
    assert_eq!(true, piece_two_good);
}
