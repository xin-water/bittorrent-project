use std::net::SocketAddr;

use crate::handshake::filter::filters::Filters;
use crate::handshake::handler;
use crate::handshake::handler::HandshakeType;
use std::io;

pub struct ListenerHandler<S> {
    opt_item: Option<HandshakeType<S>>,
}

impl<S> ListenerHandler<S> {
    pub fn new(item: (S, SocketAddr), context: &Filters) -> ListenerHandler<S> {
        let (sock, addr) = item;

        let opt_item = if handler::should_filter(Some(&addr), None, None, None, None, context) {
            None
        } else {
            Some(HandshakeType::Complete(sock, addr))
        };

        ListenerHandler { opt_item: opt_item }
    }
}

impl<S>  ListenerHandler<S> {

   pub fn poll(&mut self) -> Result<Option<HandshakeType<S>>,()> {
        Ok(self.opt_item.take())
    }
}

#[cfg(test)]
mod tests {

    use crate::handshake::handler::listener::ListenerHandler;
    use crate::handshake::handler::HandshakeType;
    use futures::Future;

    use crate::handshake::filter::filters::test_filters::{BlockAddrFilter, BlockProtocolFilter};
    use crate::handshake::filter::filters::Filters;
    use crate::handshake::Protocol;

    #[test]
    fn positive_empty_filter() {
        let exp_item = ("Testing", "0.0.0.0:0".parse().unwrap());
        let handler = ListenerHandler::new(exp_item.clone(), &Filters::new());

        let recv_enum_item = handler.wait().unwrap();

        let recv_item = match recv_enum_item {
            Some(HandshakeType::Complete(sock, addr)) => (sock, addr),
            Some(HandshakeType::Initiate(_, _)) | None => {
                panic!("Expected HandshakeType::Complete")
            }
        };

        assert_eq!(exp_item, recv_item);
    }

    #[test]
    fn positive_passes_filter() {
        let filters = Filters::new();
        filters.add_filter(BlockAddrFilter::new("1.2.3.4:5".parse().unwrap()));

        let exp_item = ("Testing", "0.0.0.0:0".parse().unwrap());
        let handler = ListenerHandler::new(exp_item.clone(), &filters);

        let recv_enum_item = handler.wait().unwrap();

        let recv_item = match recv_enum_item {
            Some(HandshakeType::Complete(sock, addr)) => (sock, addr),
            Some(HandshakeType::Initiate(_, _)) | None => {
                panic!("Expected HandshakeType::Complete")
            }
        };

        assert_eq!(exp_item, recv_item);
    }

    #[test]
    fn positive_needs_data_filter() {
        let filters = Filters::new();
        filters.add_filter(BlockProtocolFilter::new(Protocol::BitTorrent));

        let exp_item = ("Testing", "0.0.0.0:0".parse().unwrap());
        let handler = ListenerHandler::new(exp_item.clone(), &filters);

        let recv_enum_item = handler.wait().unwrap();

        let recv_item = match recv_enum_item {
            Some(HandshakeType::Complete(sock, addr)) => (sock, addr),
            Some(HandshakeType::Initiate(_, _)) | None => {
                panic!("Expected HandshakeType::Complete")
            }
        };

        assert_eq!(exp_item, recv_item);
    }

    #[test]
    fn positive_fails_filter() {
        let filters = Filters::new();
        filters.add_filter(BlockAddrFilter::new("0.0.0.0:0".parse().unwrap()));

        let exp_item = ("Testing", "0.0.0.0:0".parse().unwrap());
        let handler = ListenerHandler::new(exp_item.clone(), &filters);

        let recv_enum_item = handler.wait().unwrap();

        match recv_enum_item {
            Some(HandshakeType::Complete(_, _)) | Some(HandshakeType::Initiate(_, _)) => {
                panic!("Expected No HandshakeType")
            }
            None => (),
        }
    }
}
