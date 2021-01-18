use std::io;
use crossbeam::channel::{Sender,Receiver};
use std::io::{Error, ErrorKind};

pub trait Stream: Send {

    type Item;

    fn poll(&mut self)->io::Result<Self::Item>;
}


impl<T> Stream for  Receiver<T> where T :Send
{
    type Item = T;

    fn poll(&mut self) -> io::Result<Self::Item> {
        if let Ok(rerustle)= Receiver::recv(self){
            Ok(rerustle)
        }else {
            Err(Error::new(ErrorKind::NotFound, "Receiver recv not found"))
        }
    }
}
