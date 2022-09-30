
use crate::filter::filters::Filters;
use crate::handler;
use crate::handler::timer::HandshakeTimer;
use crate::handler::HandshakeType;
use crate::{InitiateMessage, Transport};

/// Handle the initiation of connections, which are returned as a HandshakeType.
pub fn initiator_handler<T>(
    item: InitiateMessage,
    context: &(T, Filters, HandshakeTimer),
) -> Result<Option<HandshakeType<T::Socket>>,()>
where
    T: Transport,
{
    let &(ref transport, ref filters,  ref timer) = context;

    if handler::should_filter(
        Some(item.address()),
        Some(item.protocol()),
        None,
        Some(item.hash()),
        None,
        filters,
    ) {
        Ok(None)

    } else {
        let res_connect = transport
            .connect(item.address());

       res_connect
           .map(|socket| Some(HandshakeType::Initiate(socket, item)))
           .or_else(|_| Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::Future;
    use tokio_core::reactor::Core;
    use tokio_timer;

    use btp_util::bt;
    use btp_util::bt::{InfoHash, PeerId};

    use crate::filter::filters::test_filters::{
        BlockAddrFilter, BlockPeerIdFilter, BlockProtocolFilter,
    };
    use crate::filter::filters::Filters;
    use crate::handler::timer::HandshakeTimer;
    use crate::handler::HandshakeType;
    use crate::transport::test_transports::MockTransport;
    use crate::{InitiateMessage, Protocol};

    fn any_peer_id() -> PeerId {
        [22u8; bt::PEER_ID_LEN].into()
    }

    fn any_info_hash() -> InfoHash {
        [55u8; bt::INFO_HASH_LEN].into()
    }

    #[test]
    fn positive_empty_filter() {
        let core = Core::new().unwrap();
        let exp_message = InitiateMessage::new(
            Protocol::BitTorrent,
            any_info_hash(),
            "1.2.3.4:5".parse().unwrap(),
        );
        let timer = HandshakeTimer::new(Duration::from_millis(1000));

        let recv_enum_item = super::initiator_handler(
            exp_message.clone(),
            &(MockTransport, Filters::new(), core.handle(), timer),
        )
        .wait()
        .unwrap();
        let recv_item = match recv_enum_item {
            Some(HandshakeType::Initiate(_, msg)) => msg,
            Some(HandshakeType::Complete(_, _)) | None => {
                panic!("Expected HandshakeType::Initiate")
            }
        };

        assert_eq!(exp_message, recv_item);
    }

    #[test]
    fn positive_passes_filter() {
        let core = Core::new().unwrap();
        let timer = HandshakeTimer::new( Duration::from_millis(1000));

        let filters = Filters::new();
        filters.add_filter(BlockAddrFilter::new("2.3.4.5:6".parse().unwrap()));

        let exp_message = InitiateMessage::new(
            Protocol::BitTorrent,
            any_info_hash(),
            "1.2.3.4:5".parse().unwrap(),
        );

        let recv_enum_item = super::initiator_handler(
            exp_message.clone(),
            &(MockTransport, filters, core.handle(), timer),
        )
        .wait()
        .unwrap();
        let recv_item = match recv_enum_item {
            Some(HandshakeType::Initiate(_, msg)) => msg,
            Some(HandshakeType::Complete(_, _)) | None => {
                panic!("Expected HandshakeType::Initiate")
            }
        };

        assert_eq!(exp_message, recv_item);
    }

    #[test]
    fn positive_needs_data_filter() {
        let core = Core::new().unwrap();
        let timer = HandshakeTimer::new(Duration::from_millis(1000));

        let filters = Filters::new();
        filters.add_filter(BlockPeerIdFilter::new(any_peer_id()));

        let exp_message = InitiateMessage::new(
            Protocol::BitTorrent,
            any_info_hash(),
            "1.2.3.4:5".parse().unwrap(),
        );

        let recv_enum_item = super::initiator_handler(
            exp_message.clone(),
            &(MockTransport, filters, core.handle(), timer),
        )
        .wait()
        .unwrap();
        let recv_item = match recv_enum_item {
            Some(HandshakeType::Initiate(_, msg)) => msg,
            Some(HandshakeType::Complete(_, _)) | None => {
                panic!("Expected HandshakeType::Initiate")
            }
        };

        assert_eq!(exp_message, recv_item);
    }

    #[test]
    fn positive_fails_filter() {
        let core = Core::new().unwrap();
        let timer = HandshakeTimer::new( Duration::from_millis(1000));

        let filters = Filters::new();
        filters.add_filter(BlockProtocolFilter::new(Protocol::Custom(vec![1, 2, 3, 4])));

        let exp_message = InitiateMessage::new(
            Protocol::Custom(vec![1, 2, 3, 4]),
            any_info_hash(),
            "1.2.3.4:5".parse().unwrap(),
        );

        let recv_enum_item = super::initiator_handler(
            exp_message.clone(),
            &(MockTransport, filters, core.handle(), timer),
        )
        .wait()
        .unwrap();
        match recv_enum_item {
            None => (),
            Some(HandshakeType::Initiate(_, _)) | Some(HandshakeType::Complete(_, _)) => {
                panic!("Expected No Handshake")
            }
        }
    }
}
