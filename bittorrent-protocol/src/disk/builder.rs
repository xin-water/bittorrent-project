use super::fs::FileSystem;
use crate::disk::DiskManager;
use threadpool::Builder;

const DEFAULT_BUFFER_SIZE: usize = 10;

/// `DiskManagerBuilder` for building `DiskManager`s with different settings.
pub struct DiskManagerBuilder {
    thread_pool_builder: Builder,
    buffer_capacity_size: usize,
}

impl DiskManagerBuilder {
    /// Create a new `DiskManagerBuilder`.
    pub fn new() -> DiskManagerBuilder {
        DiskManagerBuilder {
            thread_pool_builder: Builder::new(),
            buffer_capacity_size: DEFAULT_BUFFER_SIZE,
        }
    }

    /// Use a custom `Builder` for the `threadpool`.
    pub fn with_worker_config(mut self, config: Builder) -> DiskManagerBuilder {
        self.thread_pool_builder = config;
        self
    }

    /// Specify the buffer capacity for pending `IDiskMessage`s.
    pub fn with_buffer_capacity(mut self, size: usize) -> DiskManagerBuilder {
        self.buffer_capacity_size = size;
        self
    }

    /// Retrieve the `threadpool` builder.
    pub fn worker_config(&mut self) -> Builder {
        self.thread_pool_builder.clone()
    }

    /// Retrieve the sink buffer capacity.
    pub fn get_buffer_capacity(&self) -> usize {
        self.buffer_capacity_size
    }

    /// Build a `DiskManager` with the given `FileSystem`.
    pub fn build<F>(self, fs: F) -> DiskManager<F>
    where
        F: FileSystem + Send + Sync + 'static,
    {
        DiskManager::from_builder(self, fs)
    }
}
