mod memory;
pub use self::memory::block::{Block, BlockMetadata, BlockMut};

pub mod fs;
pub use self::fs::cache::file_handle::FileHandleCache;
pub use self::fs::native::{NativeFile, NativeFileSystem};
pub use self::fs::FileSystem;

pub mod message;
pub use self::message::{IDiskMessage, ODiskMessage};

mod tasks;

pub mod manager;
pub use self::manager::{DiskManager, DiskManagerSink, DiskManagerStream};

pub mod builder;
pub use self::builder::DiskManagerBuilder;

/// Both `Block` and `Torrent` error types.
pub mod error;

pub use crate::util::bt::InfoHash;
