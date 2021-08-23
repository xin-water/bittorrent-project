use futures::{SinkExt, StreamExt};
use tokio::test;
use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{
    Block, BlockMetadata, DiskManagerBuilder, FileSystem, IDiskMessage, ODiskMessage,
};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};
use bytes::BytesMut;

#[test]
async fn positive_process_block() {
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

    // Spin up a disk manager and add our created torrent to its
    let filesystem = InMemoryFileSystem::new();
    let disk_manager = DiskManagerBuilder::new().build(filesystem.clone());

    let mut process_bytes = BytesMut::new();
    process_bytes.extend_from_slice(&data_b.0[1..(50 + 1)]);

    let process_block = Block::new(
        BlockMetadata::new(metainfo_file.info().info_hash(), 1, 0, 50),
        process_bytes.freeze(),
    );

    let (mut blocking_send, mut recv) = disk_manager.into_parts();

    blocking_send
        .send(IDiskMessage::AddTorrent(metainfo_file))
        .unwrap();

    loop {
        let msg = recv.next().unwrap();

        match msg {
            ODiskMessage::TorrentAdded(_) => {
                blocking_send
                    .send(IDiskMessage::ProcessBlock(process_block.clone()))
                    .unwrap();
            }
            ODiskMessage::BlockProcessed(_) => break,
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }

    // Verify block was updated in data_b
    let mut received_file_b = filesystem.open_file(data_b.1).unwrap();
    assert_eq!(2000, filesystem.file_size(&received_file_b).unwrap());

    let mut recevied_file_b_data = vec![0u8; 2000];
    assert_eq!(
        2000,
        filesystem
            .read_file(&mut received_file_b, 0, &mut recevied_file_b_data)
            .unwrap()
    );

    let mut expected_file_b_data = vec![0u8; 2000];
    (&mut expected_file_b_data[1..(1 + 50)]).copy_from_slice(&data_b.0[1..(50 + 1)]);
    assert_eq!(expected_file_b_data, recevied_file_b_data);
}
