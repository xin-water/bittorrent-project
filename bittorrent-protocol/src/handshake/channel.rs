#![allow(unused)]

use std::sync::mpsc::{self};

/// Trait for handshakers to send data back to the client.
pub trait Channel<T: Send>: Send {
    /// Send data back to the client.
    ///
    /// Consumers will expect this method to not block.
    fn send(&mut self, data: T);
    fn clone(&mut self) ->Self;

}

impl<T: Send> Channel<T> for mpsc::Sender<T> {
    fn send(&mut self, data: T) {
        mpsc::Sender::send(self, data);
    }

    fn clone(&mut self) ->Self{
           mpsc::Sender::clone(self)
    }
}

impl<T: Send> Channel<T> for mpsc::SyncSender<T> {
    fn send(&mut self, data: T) {
        mpsc::SyncSender::try_send(self, data);
    }

    fn clone(&mut self) ->Self{
        mpsc::SyncSender::clone(self)
    }
}

