use std::io::{Read, Write};
use std::net::SocketAddr;
use crossbeam::channel::{Receiver, Sender};
use crate::filter::filters::Filters;
use crate::{CompleteMessage, Extensions, FilterDecision, InitiateMessage, LocalAddr, Protocol, Transport};
use btp_util::bt::{InfoHash, PeerId};
use crate::handler::timer::HandshakeTimer;
use crate::stream::Stream;

pub mod handshaker;
pub mod timer;
pub mod framed;

pub enum HandshakeType<S> {
    //主动握手
    Initiate(S, InitiateMessage),
    //监听握手
    Complete(S, SocketAddr),
}

pub fn loop_handler_command<S, T>(mut stream: Receiver<InitiateMessage>,
                                      transport: T,
                                      filters: Filters,
                                      sink: Sender<HandshakeType<S>>)
where
        S: Read + Write + 'static + Send ,
        T: Transport<Socket = S> + 'static + Send,
{
    std::thread::spawn(move||{
        loop {
            let item = stream.poll().unwrap();

            // 过虑判断，不应该过虑就发起链接
            if !should_filter(Some(item.address()),
                              Some(item.protocol()),
                              None,
                              Some(item.hash()),
                              None,
                              &filters
            ){
                let res_connect = transport.connect(item.address());
                if let Ok(socket) = res_connect {
                    sink.send(HandshakeType::Initiate(socket, item)).unwrap();
                }
            }
        }
    });
}

pub fn loop_handler_Listener<S,L>(mut stream: L,
                                      filters: Filters,
                                      sink: Sender<HandshakeType<S>>)
where
        S: Read + Write + 'static + Send ,
        L: Stream<Item = (S, SocketAddr) > + LocalAddr + 'static,
{
    std::thread::spawn(move||{
        loop {
            let (sock,addr) = stream.poll().unwrap();

            if !should_filter(Some(&addr), None, None, None, None, &filters) {
                sink.send(HandshakeType::Complete(sock, addr)).unwrap();
            }
        }
    });
}

pub fn loop_handler_handshake<S>(mut stream: Receiver<HandshakeType<S>>,
                                      context:(Extensions,PeerId,Filters,HandshakeTimer),
                                      sink: Sender<CompleteMessage<S>>)
    where
        S: Read + Write + 'static + Send ,
{
    std::thread::spawn(move||{
        loop {
            let item = stream.poll().unwrap();
            let opt_result = handshaker::execute_handshake(item, &context).unwrap();
            if let Some(result) = opt_result {
                sink.send(result).unwrap();
            }
        }
    });
}

/// Computes whether or not we should filter given the parameters and filters.
pub fn should_filter(
    addr: Option<&SocketAddr>,
    prot: Option<&Protocol>,
    ext: Option<&Extensions>,
    hash: Option<&InfoHash>,
    pid: Option<&PeerId>,
    filters: &Filters,
) -> bool {
    // Initially, we set all our results to pass
    let mut addr_filter = FilterDecision::Pass;
    let mut prot_filter = FilterDecision::Pass;
    let mut ext_filter = FilterDecision::Pass;
    let mut hash_filter = FilterDecision::Pass;
    let mut pid_filter = FilterDecision::Pass;

    // Choose on individual fields
    filters.access_filters(|ref_filters| {
        for ref_filter in ref_filters {
            addr_filter = addr_filter.choose(ref_filter.on_addr(addr));
            prot_filter = prot_filter.choose(ref_filter.on_prot(prot));
            ext_filter = ext_filter.choose(ref_filter.on_ext(ext));
            hash_filter = hash_filter.choose(ref_filter.on_hash(hash));
            pid_filter = pid_filter.choose(ref_filter.on_pid(pid));
        }
    });

    // Choose across the results of individual fields
    addr_filter
        .choose(prot_filter)
        .choose(ext_filter)
        .choose(hash_filter)
        .choose(pid_filter)
        == FilterDecision::Block
}
