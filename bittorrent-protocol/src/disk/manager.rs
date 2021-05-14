use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use std::sync::mpsc::{Receiver};
use threadpool::ThreadPool;

use std::pin::Pin;
use std::task::Poll;
use futures::task::Context;
use futures::{
    Future, FutureExt, Sink, SinkExt, Stream, StreamExt, TryFuture, TryFutureExt, TryStream,
    TryStreamExt,
};
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
        let sink_capacity = builder.sink_buffer_capacity();
        let stream_capacity = builder.stream_buffer_capacity();
        let cur_sink_capacity = Arc::new(AtomicUsize::new(0));
        let pool_builder = builder.worker_config();

        //let (out_send, out_recv) = tokio::sync::mpsc::channel(stream_capacity);
        let (out_send, out_recv) = std::sync::mpsc::channel();

        let context = DiskManagerContext::new(out_send, fs);

        let sink = DiskManagerSink::new(
            pool_builder.build(),
            context,
            sink_capacity,
            cur_sink_capacity.clone(),
        );
        let stream = DiskManagerStream::new(out_recv, cur_sink_capacity);

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

impl<F> Sink<IDiskMessage> for DiskManager<F>
where
    F: FileSystem + Send + Sync + 'static,
{
    type Error = ();

    fn start_send(mut self: Pin<&mut Self>, item: IDiskMessage) -> Result<(), Self::Error> {
        Pin::new(&mut self.sink).start_send(item)
    }

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink).poll_ready(cx)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.sink).poll_close(cx)
    }
}

impl<F> Stream for DiskManager<F> {
    type Item = ODiskMessage;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
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

impl<F> Sink<IDiskMessage> for DiskManagerSink<F>
where
    F: FileSystem + Send + Sync + 'static,
{
    type Error = ();

    fn start_send(self: Pin<&mut Self>, item: IDiskMessage) -> Result<(), Self::Error> {
        info!("Starting Send For DiskManagerSink With IDiskMessage");

        if self.try_submit_work() {
            info!("DiskManagerSink Submitted Work On First Attempt");
            tasks::execute_on_pool(item, self.pool.clone(), self.context.clone());

            return Ok(());
        }

        // We split the sink and stream, which means these could be polled in different event loops (I think),
        // so we need to add our task, but then try to sumbit work again, in case the receiver processed work
        // right after we tried to submit the first time.
        info!("DiskManagerSink Failed To Submit Work On First Attempt, Adding Task To Queue");
        //self.task_queue.push(task::current());

        if self.try_submit_work() {
            // Receiver will look at the queue but wake us up, even though we dont need it to now...
            info!("DiskManagerSink Submitted Work On Attempt");
            tasks::execute_on_pool(item, self.pool.clone(), self.context.clone());
            return Ok(());
        } else {
            info!("DiskManagerSink Submitted Work fail");
            Err(())
            // Ok(AsyncSink::NotReady(item))
        }
    }

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // unimplemented!()
        Poll::Ready(Ok(()))
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

impl Stream for DiskManagerStream {
    type Item = ODiskMessage;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        info!("Polling DiskManagerStream For ODiskMessage");

        match self.recv.recv() {
            res @ Ok(ODiskMessage::TorrentAdded(_))
            | res @ Ok(ODiskMessage::TorrentRemoved(_))
            | res @ Ok(ODiskMessage::TorrentSynced(_))
            | res @ Ok(ODiskMessage::BlockLoaded(_))
            | res @ Ok(ODiskMessage::BlockProcessed(_)) => {
                self.complete_work();
                Poll::Ready(Some(res.unwrap()))
            }
            other => Poll::Ready(Some(other.unwrap())),
        }
    }
}
