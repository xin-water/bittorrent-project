use super::{InMemoryFileSystem, MultiFileDirectAccessor};
use bittorrent_protocol::disk::{DiskManagerBuilder, IDiskMessage};
use bittorrent_protocol::metainfo::{Metainfo, MetainfoBuilder, PieceLength};
use futures::future::Future;
use futures::sink::Sink;
use futures::stream::Stream;
use futures::{future, AsyncSink};
use tokio::runtime::Runtime;

#[test]
fn positive_disk_manager_send_backpressure() {
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
    let (m_send, m_recv) = DiskManagerBuilder::new()
        .with_sink_buffer_capacity(1)
        .build(filesystem.clone())
        .split();

    let mut runtime = Runtime::new().unwrap();

    // Add a torrent, so our receiver has a single torrent added message buffered
    let mut m_send = runtime
        .block_on(m_send.send(IDiskMessage::AddTorrent(metainfo_file)))
        .unwrap();

    // Try to send a remove message (but it should fail)
    let (result, m_send) = runtime
        .block_on(future::lazy(|| {
            future::ok::<_, ()>((
                m_send.start_send(IDiskMessage::RemoveTorrent(info_hash)),
                m_send,
            ))
        }))
        .unwrap();

    match result {
        Ok(AsyncSink::NotReady(_)) => (),
        _ => panic!("Unexpected Result From Backpressure Test"),
    };

    // Receive from our stream to unblock the backpressure
    let m_recv = runtime
        .block_on(m_recv.into_future().map(|(_, recv)| recv).map_err(|_| ()))
        .unwrap();

    // Try to send a remove message again which should go through
    let _ = runtime
        .block_on(m_send.send(IDiskMessage::RemoveTorrent(info_hash)))
        .unwrap();

    // Receive confirmation (just so the pool doesnt panic because we ended before it could send the message back)
    let _ = runtime
        .block_on(m_recv.into_future().map(|(_, recv)| recv).map_err(|_| ()))
        .unwrap();
}
