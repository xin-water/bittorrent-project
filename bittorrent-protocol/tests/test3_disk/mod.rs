use std::cmp;
use std::collections::HashMap;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rand::Rng;
use futures::SinkExt;

use bittorrent_protocol::disk::{
    BlockMetadata, BlockMut, DiskManagerSink, DiskManagerStream, FileSystem, IDiskMessage,
    ODiskMessage,
};
use bittorrent_protocol::metainfo::{Accessor, IntoAccessor, PieceAccess};
use bittorrent_protocol::util::bt::InfoHash;

mod add_torrent;
mod complete_torrent;
mod disk_manager_send_backpressure;
mod load_block;
mod process_block;
mod remove_torrent;
mod resume_torrent;
mod start;

/// Send block with the given metadata and entire data given.
fn send_block<F, M>(
    mut blocking_send: DiskManagerSink<F>,
    data: &[u8],
    hash: InfoHash,
    piece_index: u64,
    block_offset: u64,
    block_len: usize,
    modify: M,
) where
    F: FileSystem + Send + Sync + 'static,
    M: Fn(&mut [u8]),
{
    let mut bytes = BytesMut::new();
    bytes.extend_from_slice(data);

    let mut block = BlockMut::new(
        BlockMetadata::new(hash, piece_index, block_offset, block_len),
        bytes,
    );

    modify(&mut block[..]);

    let message = IDiskMessage::ProcessBlock(block.into());

    tokio::spawn(async move{
          blocking_send.send(message).await
         }
    );
}

//----------------------------------------------------------------------------//

/// Allow us to mock out multi file torrents.
struct MultiFileDirectAccessor {
    dir: PathBuf,
    files: Vec<(Vec<u8>, PathBuf)>,
}

impl MultiFileDirectAccessor {
    pub fn new(dir: PathBuf, files: Vec<(Vec<u8>, PathBuf)>) -> MultiFileDirectAccessor {
        MultiFileDirectAccessor {
            dir: dir,
            files: files,
        }
    }
}

// TODO: Ugh, once specialization lands, we can see about having a default impl for IntoAccessor
impl IntoAccessor for MultiFileDirectAccessor {
    type Accessor = MultiFileDirectAccessor;

    fn into_accessor(self) -> io::Result<MultiFileDirectAccessor> {
        Ok(self)
    }
}

impl Accessor for MultiFileDirectAccessor {
    fn access_directory(&self) -> Option<&Path> {
        // Do not just return the option here, unwrap it and put it in
        // another Option (since we know this is a multi file torrent)
        Some(self.dir.as_ref())
    }

    fn access_metadata<C>(&self, mut callback: C) -> io::Result<()>
    where
        C: FnMut(u64, &Path),
    {
        for &(ref buffer, ref path) in self.files.iter() {
            callback(buffer.len() as u64, &*path)
        }

        Ok(())
    }

    fn access_pieces<C>(&self, mut callback: C) -> io::Result<()>
    where
        C: for<'a> FnMut(PieceAccess<'a>) -> io::Result<()>,
    {
        for &(ref buffer, _) in self.files.iter() {
            callback(PieceAccess::Compute(&mut &buffer[..]))?
        }

        Ok(())
    }
}

//----------------------------------------------------------------------------//

/// Allow us to mock out the file system.
#[derive(Clone)]
struct InMemoryFileSystem {
    files: Arc<Mutex<HashMap<PathBuf, Vec<u8>>>>,
}

impl InMemoryFileSystem {
    pub fn new() -> InMemoryFileSystem {
        InMemoryFileSystem {
            files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn run_with_lock<C, R>(&self, call: C) -> R
    where
        C: FnOnce(&mut HashMap<PathBuf, Vec<u8>>) -> R,
    {
        let mut lock_files = self.files.lock().unwrap();

        call(&mut *lock_files)
    }
}

struct InMemoryFile {
    path: PathBuf,
}

impl FileSystem for InMemoryFileSystem {
    type File = InMemoryFile;

    fn open_file<P>(&self, path: P) -> io::Result<Self::File>
    where
        P: AsRef<Path> + Send + 'static,
    {
        let file_path = path.as_ref().to_path_buf();

        self.run_with_lock(|files| {
            if !files.contains_key(&file_path) {
                files.insert(file_path.clone(), Vec::new());
            }
        });

        Ok(InMemoryFile { path: file_path })
    }

    fn sync_file<P>(&self, _path: P) -> io::Result<()>
    where
        P: AsRef<Path> + Send + 'static,
    {
        Ok(())
    }

    fn file_size(&self, file: &Self::File) -> io::Result<u64> {
        self.run_with_lock(|files| {
            files
                .get(&file.path)
                .map(|file| file.len() as u64)
                .ok_or(io::Error::new(io::ErrorKind::NotFound, "File Not Found"))
        })
    }

    fn read_file(
        &self,
        file: &mut Self::File,
        offset: u64,
        buffer: &mut [u8],
    ) -> io::Result<usize> {
        self.run_with_lock(|files| {
            files
                .get(&file.path)
                .map(|file_buffer| {
                    let cast_offset = offset as usize;
                    let bytes_to_copy = cmp::min(file_buffer.len() - cast_offset, buffer.len());
                    let bytes = &file_buffer[cast_offset..(bytes_to_copy + cast_offset)];

                    buffer.clone_from_slice(bytes);

                    bytes_to_copy
                })
                .ok_or(io::Error::new(io::ErrorKind::NotFound, "File Not Found"))
        })
    }

    fn write_file(&self, file: &mut Self::File, offset: u64, buffer: &[u8]) -> io::Result<usize> {
        self.run_with_lock(|files| {
            files
                .get_mut(&file.path)
                .map(|file_buffer| {
                    let cast_offset = offset as usize;

                    let last_byte_pos = cast_offset + buffer.len();
                    if last_byte_pos > file_buffer.len() {
                        file_buffer.resize(last_byte_pos, 0);
                    }

                    let bytes_to_copy = cmp::min(file_buffer.len() - cast_offset, buffer.len());

                    if bytes_to_copy != 0 {
                        file_buffer[cast_offset..(cast_offset + bytes_to_copy)]
                            .clone_from_slice(buffer);
                    }

                    // TODO: If the file is full, this will return zero, we should also simulate io::ErrorKind::WriteZero
                    bytes_to_copy
                })
                .ok_or(io::Error::new(io::ErrorKind::NotFound, "File Not Found"))
        })
    }
}

/// Generate buffer of size random bytes.
fn random_buffer(size: usize) -> Vec<u8> {
    let mut buffer = vec![0u8; size];

    let mut rng = rand::weak_rng();
    for i in 0..size {
        buffer[i] = rng.gen();
    }

    buffer
}

#[test]
pub fn my_print() {
    println!("test disk");
}
