use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{
    Block, BlockMetadata, BlockMut, DiskManagerBuilder, IDiskMessage, ODiskMessage,
};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};
use bytes::BytesMut;

#[test]
fn positive_load_block() {
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

    let mut process_block_bytemut = BytesMut::new();
    process_block_bytemut.extend_from_slice(&data_b.0[1..(50 + 1)]);

    let process_block = Block::new(
        BlockMetadata::new(metainfo_file.info().info_hash(), 1, 0, 50),
        process_block_bytemut.freeze(),
    );

    let mut load_block_bytemut = BytesMut::with_capacity(50);
    load_block_bytemut.extend_from_slice(&[0u8; 50]);

    let load_block = BlockMut::new(
        BlockMetadata::new(metainfo_file.info().info_hash(), 1, 0, 50),
        load_block_bytemut,
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
            ODiskMessage::BlockProcessed(block) => {
                println!("pblock:\n{:?}", block);

                blocking_send
                    .send(IDiskMessage::LoadBlock(load_block.clone()))
                    .unwrap();
            }
            ODiskMessage::BlockLoaded(block) => {
                println!("lblock:\n{:?}", block);
                break;
            }
            unexpected @ _ => panic!("Unexpected Message: {:?}", unexpected),
        };
    }
}
