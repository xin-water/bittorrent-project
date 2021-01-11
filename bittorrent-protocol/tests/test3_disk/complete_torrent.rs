use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage, ODiskMessage};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};

#[test]
fn positive_complete_torrent() {
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

    // Spin up a disk manager and add our created torrent to it
    let filesystem = InMemoryFileSystem::new();
    let disk_manager = DiskManagerBuilder::new().build(filesystem.clone());

    let (mut blocking_send, mut recv) = disk_manager.into_parts();

    blocking_send
        .send(IDiskMessage::AddTorrent(metainfo_file.clone()))
        .unwrap();

    let good_pieces = 0;

    // Run a core loop until we get the TorrentAdded message
    loop {
        match recv.next().unwrap() {
            ODiskMessage::TorrentAdded(_) => break,
            ODiskMessage::FoundGoodPiece(_, _) => good_pieces + 1,
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }

    // Make sure we have no good pieces
    assert_eq!(0, good_pieces);

    // Send a couple blocks that are known to be good, then one bad block
    let mut files_bytes = Vec::new();
    files_bytes.extend_from_slice(&data_a.0);
    files_bytes.extend_from_slice(&data_b.0);

    // Send piece 0 with a bad last block
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
        |bytes| {
            bytes[0] = !bytes[0];
        },
    );

    // Send piece 1 with good blocks
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

    // Send piece 2 with good blocks
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

    // Verify that piece 0 is bad, but piece 1 and 2 are good
    let mut piece_zero_good = false;
    let mut piece_one_good = false;
    let mut piece_two_good = false;
    let mut messages_recvd = 0;

    loop {
        let msg = recv.next().unwrap();

        // Map BlockProcessed to a None piece index so we don't update our state
        let (opt_piece_index, new_value) = match msg {
            ODiskMessage::FoundGoodPiece(_, index) => (Some(index), true),
            ODiskMessage::FoundBadPiece(_, index) => (Some(index), false),
            ODiskMessage::BlockProcessed(_) => (None, false),
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };

        let (piece_zero_good, piece_one_good, piece_two_good) = match opt_piece_index {
            None => (piece_zero_good, piece_one_good, piece_two_good),
            Some(0) => (new_value, piece_one_good, piece_two_good),
            Some(1) => (piece_zero_good, new_value, piece_two_good),
            Some(2) => (piece_zero_good, piece_one_good, new_value),
            Some(x) => panic!("Unexpected Index {:?}", x),
        };

        // One message for each block (8 blocks), plus 3 messages for bad/good
        if messages_recvd == (8 + 3) {
            break;
        }
    }

    // Assert whether or not pieces were good
    assert_eq!(false, piece_zero_good);
    assert_eq!(true, piece_one_good);
    assert_eq!(true, piece_two_good);

    // Resend piece 0 with good blocks
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

    // Verify that piece 0 is now good
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

        piece_zero_good = match opt_piece_index {
            None => false,
            Some(0) => new_value,
            Some(x) => panic!("Unexpected Index {:?}", x),
        };

        // One message for each block (3 blocks), plus 1 messages for bad/good
        if messages_recvd == (3 + 1) {
            break;
        }
    }
    // Assert whether or not piece was good
    assert_eq!(true, piece_zero_good);
}
