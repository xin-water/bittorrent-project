use std::net::SocketAddr;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

use btp_util::bt::PeerId;

use crate::socket::framed::FramedHandshake;
use crate::message::handshake::HandshakeMessage;
use crate::filter::filters::Filters;
use crate::handler;
use crate::{CompleteMessage, Extensions, InitiateMessage};


pub(crate) async fn initiate_handshake<S>(
    sock: S,
    init_msg: InitiateMessage,
    ext: Extensions,
    pid: PeerId,
    filters: Arc<Filters>,
) -> Result<Option<CompleteMessage<S>>, ()>
where
    S: AsyncRead + AsyncWrite + 'static + Unpin,
{
    let mut framed = FramedHandshake::new(sock);

    let (prot, hash, addr) = init_msg.into_parts();
    let handshake_msg = HandshakeMessage::from_parts(prot.clone(), ext, hash, pid);

        framed.send(handshake_msg).await;

    if let Ok(Some(msg))= framed.read().await{
        let (remote_prot, remote_ext, remote_hash, remote_pid) = msg.into_parts();
        let socket = framed.into_inner();

        // Check that it responds with the same hash and protocol, also check our filters
        if remote_hash == hash
            && remote_prot == prot
            && !handler::should_filter(Some(&addr), Some(&remote_prot), Some(&remote_ext), Some(&remote_hash), Some(&remote_pid), &filters)
        {

            return  Ok(Some(CompleteMessage::new(prot, ext.union(&remote_ext), hash, remote_pid, addr, socket)));
        }
        return Ok(None);
    }

    Ok(None)
}

pub async fn complete_handshake<S>(
    sock: S,
    addr: SocketAddr,
    ext: Extensions,
    pid: PeerId,
    filters: Arc<Filters>,
) -> Result<Option<CompleteMessage<S>>,()>
where
    S: AsyncRead + AsyncWrite + 'static + Unpin,
{
    let mut framed = FramedHandshake::new(sock);

    if let Ok(Some(msg))= framed.read().await{
        let (remote_prot, remote_ext, remote_hash, remote_pid) = msg.into_parts();

        // Check our filters
        if !handler::should_filter(Some(&addr), Some(&remote_prot), Some(&remote_ext), Some(&remote_hash), Some(&remote_pid), &filters)
        {
            let handshake_msg = HandshakeMessage::from_parts(remote_prot.clone(), ext, remote_hash, pid);
            framed.send(handshake_msg).await;

            let socket = framed.into_inner();
            return Ok(Some(CompleteMessage::new(
                remote_prot,
                ext.union(&remote_ext),
                remote_hash,
                remote_pid,
                addr,
                socket,
            )));
        }
        return Err(());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::time::Duration;

    use futures::future::{self, Future};
    use tokio_timer;

    use btp_util::bt;
    use btp_util::bt::{InfoHash, PeerId};

    use crate::message::handshake::HandshakeMessage;
    use crate::filter::filters::Filters;
    use crate::handler::timer::HandshakeTimer;
    use crate::message::extensions;
    use crate::{Extensions, InitiateMessage, Protocol};

    fn any_peer_id() -> PeerId {
        [22u8; bt::PEER_ID_LEN].into()
    }

    fn any_other_peer_id() -> PeerId {
        [33u8; bt::PEER_ID_LEN].into()
    }

    fn any_info_hash() -> InfoHash {
        [55u8; bt::INFO_HASH_LEN].into()
    }

    fn any_extensions() -> Extensions {
        [255u8; extensions::NUM_EXTENSION_BYTES].into()
    }

    fn any_handshake_timer() -> HandshakeTimer {
        HandshakeTimer::new( Duration::from_millis(100))
    }

    #[test]
    fn positive_initiate_handshake() {
        let remote_pid = any_peer_id();
        let remote_addr = "1.2.3.4:5".parse().unwrap();
        let remote_protocol = Protocol::BitTorrent;
        let remote_hash = any_info_hash();
        let remote_message = HandshakeMessage::from_parts(
            remote_protocol,
            any_extensions(),
            remote_hash,
            remote_pid,
        );

        // Setup our buffer so that the first portion is zeroed out (so our function can write to it), and the second half is our
        // serialized message (so our function can read from it).
        let mut writer = Cursor::new(vec![0u8; remote_message.write_len() * 2]);
        writer.set_position(remote_message.write_len() as u64);

        // Write out message to the second half of the buffer
        remote_message.write_bytes(&mut writer).unwrap();
        writer.set_position(0);

        let init_hash = any_info_hash();
        let init_prot = Protocol::BitTorrent;
        let init_message = InitiateMessage::new(init_prot.clone(), init_hash, remote_addr);

        let init_ext = any_extensions();
        let init_pid = any_other_peer_id();
        let init_filters = Filters::new();
        let init_timer = any_handshake_timer();

        // Wrap in lazy since we can call wait on non sized types...
        let complete_message = future::lazy(|| {
            super::initiate_handshake(
                writer,
                init_message,
                init_ext,
                init_pid,
                init_filters,
                init_timer,
            )
        })
        .wait()
        .unwrap()
        .unwrap();

        assert_eq!(init_prot, *complete_message.protocol());
        assert_eq!(init_ext, *complete_message.extensions());
        assert_eq!(init_hash, *complete_message.hash());
        assert_eq!(remote_pid, *complete_message.peer_id());
        assert_eq!(remote_addr, *complete_message.address());

        let sent_message = HandshakeMessage::from_bytes(
            &complete_message.socket().get_ref()[..remote_message.write_len()],
        )
        .unwrap()
        .1;
        let local_message = HandshakeMessage::from_parts(init_prot, init_ext, init_hash, init_pid);

        let recv_message = HandshakeMessage::from_bytes(
            &complete_message.socket().get_ref()[remote_message.write_len()..],
        )
        .unwrap()
        .1;

        assert_eq!(local_message, sent_message);
        assert_eq!(remote_message, recv_message);
    }

    #[test]
    fn positive_complete_handshake() {
        let remote_pid = any_peer_id();
        let remote_addr = "1.2.3.4:5".parse().unwrap();
        let remote_protocol = Protocol::BitTorrent;
        let remote_hash = any_info_hash();
        let remote_message = HandshakeMessage::from_parts(
            Protocol::BitTorrent,
            any_extensions(),
            remote_hash,
            remote_pid,
        );

        // Setup our buffer so that the second portion is zeroed out (so our function can write to it), and the first half is our
        // serialized message (so our function can read from it).
        let mut writer = Cursor::new(vec![0u8; remote_message.write_len() * 2]);

        // Write out message to the first half of the buffer
        remote_message.write_bytes(&mut writer).unwrap();
        writer.set_position(0);

        let comp_ext = any_extensions();
        let comp_pid = any_other_peer_id();
        let comp_filters = Filters::new();
        let comp_timer = any_handshake_timer();

        // Wrap in lazy since we can call wait on non sized types...
        let complete_message = future::lazy(|| {
            super::complete_handshake(
                writer,
                remote_addr,
                comp_ext,
                comp_pid,
                comp_filters,
                comp_timer,
            )
        })
        .wait()
        .unwrap()
        .unwrap();

        assert_eq!(remote_protocol, *complete_message.protocol());
        assert_eq!(comp_ext, *complete_message.extensions());
        assert_eq!(remote_hash, *complete_message.hash());
        assert_eq!(remote_pid, *complete_message.peer_id());
        assert_eq!(remote_addr, *complete_message.address());

        let sent_message = HandshakeMessage::from_bytes(
            &complete_message.socket().get_ref()[remote_message.write_len()..],
        )
        .unwrap()
        .1;
        let local_message =
            HandshakeMessage::from_parts(remote_protocol, comp_ext, remote_hash, comp_pid);

        let recv_message = HandshakeMessage::from_bytes(
            &complete_message.socket().get_ref()[..remote_message.write_len()],
        )
        .unwrap()
        .1;

        assert_eq!(local_message, sent_message);
        assert_eq!(remote_message, recv_message);
    }
}
