use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{
    Block, BlockMetadata, DiskManagerBuilder, IDiskMessage, ODiskMessage,
};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};
use bytes::BytesMut;

#[test]
fn positive_remove_torrent() {
    // Create some "files" as random bytes
    let data_a = (super::random_buffer(50), "/path/to/file/a".into());
    let data_b = (super::random_buffer(2000), "/path/to/file/b".into());
    let data_c = (super::random_buffer(0), "/path/to/file/c".into());

    // Create our accessor for our in memory files and create a torrent file for them
    let files_accessor = MultiFileDirectAccessor::new(
        "/my/downloads/".into(),
        vec![data_a.clone(), data_b.clone(), data_c.clone()],
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
        .send(IDiskMessage::AddTorrent(metainfo_file))
        .unwrap();

    // Verify that zero pieces are marked as good
    let mut good_pieces = 0;
    loop {
        let msg = recv.next().unwrap();

        match msg {
            ODiskMessage::TorrentAdded(_) => {
                blocking_send
                    .send(IDiskMessage::RemoveTorrent(info_hash))
                    .unwrap();
            }
            ODiskMessage::TorrentRemoved(_) => break,
            ODiskMessage::FoundGoodPiece(_, _) => {
                good_pieces += 1;
            }
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }

    assert_eq!(0, good_pieces);

    let mut process_bytes = BytesMut::new();
    process_bytes.extend_from_slice(&data_a.0[0..50]);

    let process_block = Block::new(
        BlockMetadata::new(info_hash, 0, 0, 50),
        process_bytes.freeze(),
    );

    blocking_send
        .send(IDiskMessage::ProcessBlock(process_block))
        .unwrap();

    loop {
        let msg = recv.next().unwrap();
        match msg {
            ODiskMessage::ProcessBlockError(_, _) => break,
            unexpected => panic!("Unexpected Message: {:?}", unexpected),
        };
    }
}
