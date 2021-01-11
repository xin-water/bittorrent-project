use bytes::BytesMut;
use rand::{self, Rng};
use std::fs;

use bittorrent_protocol::metainfo::{DirectAccessor, Metainfo, MetainfoBuilder, PieceLength};
use bittorrent_protocol::util::bt::InfoHash;

use bittorrent_protocol::disk::FileHandleCache;
use bittorrent_protocol::disk::{
    Block, BlockMetadata, DiskManagerBuilder, FileSystem, IDiskMessage, ODiskMessage,
};
use bittorrent_protocol::disk::{DiskManagerSink, DiskManagerStream, NativeFileSystem};

/// Set to true if you are playing around with anything that could affect file
/// sizes for an existing or new benchmarks. As a precaution, if the disk manager
/// sees an existing file with a different size but same name as one of the files
/// in the torrent, it wont touch it and a TorrentError will be generated.
const WIPE_DATA_DIR: bool = false;

// TODO: Benchmark multi file torrents!!!

/// Generates a torrent with a single file of the given length.
///
/// Returns both the torrent file, as well as the (random) data of the file.
fn generate_single_file_torrent(piece_len: usize, file_len: usize) -> (Metainfo, Vec<u8>) {
    let mut rng = rand::weak_rng();

    let file_bytes: Vec<u8> = rng.gen_iter().take(file_len).collect();

    let metainfo_bytes = {
        let accessor = DirectAccessor::new("benchmark_file", &file_bytes[..]);

        MetainfoBuilder::new()
            .set_piece_length(PieceLength::Custom(piece_len))
            .build(1, accessor, |_| ())
            .unwrap()
    };
    let metainfo = Metainfo::from_bytes(&metainfo_bytes).unwrap();

    (metainfo, file_bytes)
}

/// Adds the given metainfo file to the given sender, and waits for the added notification.
fn add_metainfo_file<F>(
    metainfo: Metainfo,
    block_send: &mut DiskManagerSink<F>,
    block_recv: &mut DiskManagerStream,
) where
    F: FileSystem + Send + Sync + 'static,
{
    (*block_send)
        .send(IDiskMessage::AddTorrent(metainfo))
        .unwrap();

    for res_message in (*block_recv).next() {
        match res_message {
            ODiskMessage::TorrentAdded(_) => {
                break;
            }
            ODiskMessage::FoundGoodPiece(_, _) => (),
            _ => panic!("Didn't Receive TorrentAdded"),
        }
    }
}

/// Pushes the given bytes as piece blocks to the given sender, and blocks until all notifications
/// of the blocks being processed have been received (does not check piece messages).
fn process_blocks<F>(
    piece_length: usize,
    block_length: usize,
    hash: InfoHash,
    bytes: &[u8],
    mut block_send: DiskManagerSink<F>,
    mut block_recv: DiskManagerStream,
) where
    F: FileSystem + Send + Sync + 'static,
{
    let mut blocks_sent = 0;

    for (piece_index, piece) in bytes.chunks(piece_length).enumerate() {
        for (block_index, block) in piece.chunks(block_length).enumerate() {
            let block_offset = block_index * block_length;
            let mut bytes = BytesMut::new();
            bytes.extend_from_slice(block);

            let block = Block::new(
                BlockMetadata::new(hash, piece_index as u64, block_offset as u64, block.len()),
                bytes.freeze(),
            );

            block_send.send(IDiskMessage::ProcessBlock(block)).unwrap();
            blocks_sent += 1;
        }
    }
    loop {
        match block_recv.next().unwrap() {
            ODiskMessage::BlockProcessed(_) => blocks_sent -= 1,
            ODiskMessage::FoundGoodPiece(_, _) => (),
            ODiskMessage::FoundBadPiece(_, _) => (),
            _ => panic!("Unexpected Message Received In process_blocks"),
        }

        if blocks_sent == 0 {
            break;
        }
    }
}

/// Benchmarking method to setup a torrent file with the given attributes, and benchmark the block processing code.
fn bench_process_file_with_fs<F>(
    piece_length: usize,
    block_length: usize,
    file_length: usize,
    fs: F,
) where
    F: FileSystem + Send + Sync + 'static,
{
    let (metainfo, bytes) = generate_single_file_torrent(piece_length, file_length);
    let info_hash = metainfo.info().info_hash();

    let disk_manager = DiskManagerBuilder::new()
        .with_buffer_capacity(1000000)
        .build(fs);

    let (mut d_send, mut d_recv) = disk_manager.into_parts();

    add_metainfo_file(metainfo, &mut d_send, &mut d_recv);

    process_blocks(
        piece_length,
        block_length,
        info_hash,
        &bytes[..],
        d_send.clone(),
        d_recv,
    );
}

#[test]
fn bench_native_fs_1_mb_pieces_128_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 128 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_128_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = NativeFileSystem::with_directory(data_directory);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}

#[test]
fn bench_native_fs_1_mb_pieces_16_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 16 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_16_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = NativeFileSystem::with_directory(data_directory);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}

#[test]
fn bench_native_fs_1_mb_pieces_2_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 2 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_2_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = NativeFileSystem::with_directory(data_directory);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}

#[test]
fn bench_file_handle_cache_fs_1_mb_pieces_128_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 128 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_128_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = FileHandleCache::new(NativeFileSystem::with_directory(data_directory), 1);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}

#[test]
fn bench_file_handle_cache_fs_1_mb_pieces_16_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 16 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_16_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = FileHandleCache::new(NativeFileSystem::with_directory(data_directory), 1);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}

#[test]
fn bench_file_handle_cache_fs_1_mb_pieces_2_kb_blocks() {
    let piece_length = 1 * 1024 * 1024;
    let block_length = 2 * 1024;
    let file_length = 2 * 1024 * 1024;
    let data_directory = "tests_data/bench_native_fs_1_mb_pieces_2_kb_blocks";

    if WIPE_DATA_DIR {
        let _ = fs::remove_dir_all(data_directory);
    }
    let filesystem = FileHandleCache::new(NativeFileSystem::with_directory(data_directory), 1);

    bench_process_file_with_fs(piece_length, block_length, file_length, filesystem);
}
