use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use std::sync::mpsc::{Receiver};
use threadpool::ThreadPool;

use crate::disk::tasks;
use crate::disk::tasks::context::DiskManagerContext;
use crate::disk::{DiskManagerBuilder, FileSystem, IDiskMessage, ODiskMessage};

/// `DiskManager` object which handles the storage of `Blocks` to the `FileSystem`.
pub struct DiskManager<F> {
    sink: DiskManagerSink<F>,
    stream: DiskManagerStream,
}

impl<F> DiskManager<F> {
    /// Create a `DiskManager` from the given `DiskManagerBuilder`.
    pub fn from_builder(mut builder: DiskManagerBuilder, fs: F) -> DiskManager<F> {
        let buffer_capacity = builder.get_buffer_capacity();
        let cur_buffer_capacity = Arc::new(AtomicUsize::new(0));
        let pool_builder = builder.worker_config();

        //let (out_send, out_recv) = tokio::sync::mpsc::channel(stream_capacity);
        let (out_send, out_recv) = std::sync::mpsc::channel();

        let context = DiskManagerContext::new(out_send, fs);

        let sink = DiskManagerSink::new(
            pool_builder.build(),
            context,
            buffer_capacity,
            cur_buffer_capacity.clone(),
        );
        let stream = DiskManagerStream::new(out_recv, cur_buffer_capacity);

        DiskManager {
            sink: sink,
            stream: stream,
        }
    }

    /// Break the `DiskManager` into a sink and stream.
    ///
    /// The returned sink implements `Clone`.
    pub fn into_parts(self) -> (DiskManagerSink<F>, DiskManagerStream) {
        (self.sink, self.stream)
    }
}
//----------------------------------------------------------------------------//

/// `DiskManagerSink` which is the sink portion of a `DiskManager`.
pub struct DiskManagerSink<F> {
    pool: ThreadPool,
    context: DiskManagerContext<F>,
    max_capacity: usize,
    cur_capacity: Arc<AtomicUsize>,
}

impl<F> Clone for DiskManagerSink<F> {
    fn clone(&self) -> DiskManagerSink<F> {
        DiskManagerSink {
            pool: self.pool.clone(),
            context: self.context.clone(),
            max_capacity: self.max_capacity,
            cur_capacity: self.cur_capacity.clone(),
        }
    }
}

impl<F> DiskManagerSink<F> {
    fn new(
        pool: ThreadPool,
        context: DiskManagerContext<F>,
        max_capacity: usize,
        cur_capacity: Arc<AtomicUsize>,
    ) -> DiskManagerSink<F> {
        DiskManagerSink {
            pool: pool,
            context: context,
            max_capacity: max_capacity,
            cur_capacity: cur_capacity,
        }
    }

    fn try_submit_work(&self) -> bool {
        let cur_capacity = self.cur_capacity.fetch_add(1, Ordering::SeqCst);

        if cur_capacity < self.max_capacity {
            true
        } else {
            self.cur_capacity.fetch_sub(1, Ordering::SeqCst);

            false
        }
    }
}

impl<F> DiskManagerSink<F>
where
    F: FileSystem + Send + Sync + 'static,
{
    pub fn send(self: &mut Self, item: IDiskMessage) -> Result<(), IDiskMessage> {
        info!("Starting Send For DiskManagerSink With IDiskMessage");

        if self.try_submit_work() {
            // Receiver will look at the queue but wake us up, even though we dont need it to now...
            info!("DiskManagerSink Submitted Work On Attempt");
            tasks::execute_on_pool(item, self.pool.clone(), self.context.clone());
            return Ok(());
        } else {
            info!("DiskManagerSink Submitted Work fail");
            Err(item)
        }
    }
}

//----------------------------------------------------------------------------//

/// `DiskManagerStream` which is the stream portion of a `DiskManager`.
pub struct DiskManagerStream {
    recv: Receiver<ODiskMessage>,
    cur_capacity: Arc<AtomicUsize>,
}

impl DiskManagerStream {
    fn new(recv: Receiver<ODiskMessage>, cur_capacity: Arc<AtomicUsize>) -> DiskManagerStream {
        DiskManagerStream {
            recv: recv,
            cur_capacity: cur_capacity,
        }
    }

    fn complete_work(&self) {
        self.cur_capacity.fetch_sub(1, Ordering::SeqCst);
    }
}

impl DiskManagerStream {
    pub fn next(self: &mut Self) -> Option<ODiskMessage> {
        info!("Polling DiskManagerStream For ODiskMessage");

        match self.recv.recv() {
            res @ Ok(ODiskMessage::TorrentAdded(_))
            | res @ Ok(ODiskMessage::TorrentRemoved(_))
            | res @ Ok(ODiskMessage::TorrentSynced(_))
            | res @ Ok(ODiskMessage::BlockLoaded(_))
            | res @ Ok(ODiskMessage::BlockProcessed(_)) => {
                self.complete_work();
                Some(res.unwrap())
            }
            Err(_x) => {
                panic!("DiskManagerStream recv err");
            }

            other => Some(other.unwrap()),
        }
    }
}
