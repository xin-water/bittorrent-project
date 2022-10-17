use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc};

use threadpool::ThreadPool;

use std::pin::Pin;
use std::task::Poll;
use futures::task::Context;
use futures::{
    Future, FutureExt, Sink, SinkExt, Stream, StreamExt, TryFuture, TryFutureExt, TryStream,
    TryStreamExt,
};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::SendError;
use crate::tasks;
use crate::tasks::context::DiskManagerContext;
use crate::{DiskManagerBuilder, FileSystem, IDiskMessage, ODiskMessage};
use crate::tasks::start_disk_task;

/// `DiskManager` object which handles the storage of `Blocks` to the `FileSystem`.
pub struct DiskManager {
    sink: DiskManagerSink,
    stream: DiskManagerStream,
}

impl DiskManager {
    /// Create a `DiskManager` from the given `DiskManagerBuilder`.
    pub fn from_builder<F>(mut builder: DiskManagerBuilder, fs: F) -> DiskManager
    where F:std::marker::Sync + std::marker::Send + 'static,
          F:FileSystem
    {

        let sink_capacity = builder.sink_buffer_capacity();
        let stream_capacity = builder.stream_buffer_capacity();

        let (tx,rx) = start_disk_task(sink_capacity,stream_capacity,fs);

        let sink = DiskManagerSink::new(tx);
        let stream = DiskManagerStream::new(rx);

        DiskManager {
            sink: sink,
            stream: stream,
        }
    }

    pub async fn send(&mut self, msg: IDiskMessage) -> Result<(), SendError<IDiskMessage>> {
        self.sink.tx.send(msg).await
    }

    pub async fn recv(&mut self) -> Option<ODiskMessage> {
        self.stream.recv.recv().await
    }

    pub fn build() -> DiskManagerBuilder {
        DiskManagerBuilder::new()
    }

    /// Break the `DiskManager` into a sink and stream.
    ///
    /// The returned sink implements `Clone`.
    pub fn into_parts(self) -> (DiskManagerSink, DiskManagerStream) {
        (self.sink, self.stream)
    }
}

//----------------------------------------------------------------------------//

/// `DiskManagerSink` which is the sink portion of a `DiskManager`.
pub struct DiskManagerSink {
   tx: mpsc::Sender<IDiskMessage>
}

impl Clone for DiskManagerSink {
    fn clone(&self) -> DiskManagerSink {
        DiskManagerSink {
            tx: self.tx.clone(),
        }
    }
}

impl  DiskManagerSink {
    fn new(
      tx: mpsc::Sender<IDiskMessage>
    ) -> DiskManagerSink {
        DiskManagerSink {
          tx
        }
    }


    pub async fn send(&mut self, msg: IDiskMessage) -> Result<(), SendError<IDiskMessage>> {
        self.tx.send(msg).await
    }
}




//----------------------------------------------------------------------------//

/// `DiskManagerStream` which is the stream portion of a `DiskManager`.
pub struct DiskManagerStream {
    recv: mpsc::Receiver<ODiskMessage>,
}

impl DiskManagerStream {
    fn new(recv: mpsc::Receiver<ODiskMessage>) -> DiskManagerStream {
        DiskManagerStream {
            recv: recv
        }
    }

    pub async fn recv(&mut self) -> Option<ODiskMessage> {
        self.recv.recv().await
    }
}