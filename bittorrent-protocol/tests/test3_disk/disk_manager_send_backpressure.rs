use futures::{SinkExt, StreamExt};
use tokio::test;
use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};

#[test]
async fn positive_disk_manager_send_backpressure() {
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
    let (mut m_send, mut m_recv) = DiskManagerBuilder::new()
        .with_buffer_capacity(1)
        .build(filesystem.clone())
        .into_parts();

    // Add a torrent, so our receiver has a single torrent added message buffered
    m_send
        .send(IDiskMessage::AddTorrent(metainfo_file))
        .unwrap();

    // Try to send a remove message again which should go through
    m_send.send(IDiskMessage::RemoveTorrent(info_hash)).unwrap();
}
