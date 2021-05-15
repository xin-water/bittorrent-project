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

        //let (out_send, out_recv) = tokio::sync::mpsc::channel(stream_capacity);
        let (out_send, out_recv) = std::sync::mpsc::channel();

        let context = DiskManagerContext::new(out_send, fs);

        let sink = DiskManagerSink::new(
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

//----------------------------------------------------------------------------//

/// `DiskManagerSink` which is the sink portion of a `DiskManager`.
pub struct DiskManagerSink<F> {
    context: DiskManagerContext<F>,
    max_capacity: usize,
    cur_capacity: Arc<AtomicUsize>,
}

impl<F> Clone for DiskManagerSink<F> {
    fn clone(&self) -> DiskManagerSink<F> {
        DiskManagerSink {
            context: self.context.clone(),
            max_capacity: self.max_capacity,
            cur_capacity: self.cur_capacity.clone(),
        }
    }
}

impl<F> DiskManagerSink<F> {
    fn new(
        context: DiskManagerContext<F>,
        max_capacity: usize,
        cur_capacity: Arc<AtomicUsize>,
    ) -> DiskManagerSink<F> {
        DiskManagerSink {
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

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: IDiskMessage) -> Result<(), Self::Error> {

        if self.try_submit_work() {
            debug!("DiskManagerSink Submitted Work On First Attempt");
            tasks::execute_on_pool(item, self.context.clone());
            return Ok(());
        }else {
            debug!("DiskManagerSink Submitted Work fail");
            Err(())
        }
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
        debug!("Polling DiskManagerStream For ODiskMessage");

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
