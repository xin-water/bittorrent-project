use std::net::SocketAddr;
use crossbeam::channel::{Sender};
use crate::filter::filters::Filters;
use crate::{Extensions, FilterDecision, InitiateMessage, Protocol};
use btp_util::bt::{InfoHash, PeerId};
use crate::stream::Stream;

pub mod handshaker;
pub mod initiator;
pub mod listener;
pub mod timer;

pub enum HandshakeType<S> {
    Initiate(S, InitiateMessage),
    Complete(S, SocketAddr),
}

enum LoopError {
    Terminate,
    Recoverable,
}

/// Create loop for feeding the handler with the items coming from the stream, and forwarding the result to the sink.
///
/// If the stream is used up, or an error is propogated from any of the elements, the loop will terminate.
pub fn loop_handler<M, C, H, R>(mut stream:M, context: C, mut handler: H, sink: Sender<R>)
where
    M: Stream + 'static + Send,
    C: 'static + Send,
    H: FnMut(M::Item, &C) -> Result<Option<R>,()> + 'static + Send ,
    R: 'static + Send ,
{
    std::thread::spawn(move||{
        loop {
            // We will terminate the loop if, the stream gives us an error, the stream gives us None, the handler gives
            // us an error, or the sink gives us an error. If the handler gives us Ok(None), we will map that to a
            // recoverable error (since our Ok(Some) result would have to continue with its own future, we hijack
            // the error to store an immediate value). We finally map any recoverable errors back to an Ok value
            // so we can continue with the loop in that case.
            let reruslt = stream
                .poll()
                .map_err(|_| LoopError::Terminate)
                .and_then(|item| {
                    let result = handler(item, &context);
                    result
                        .map_err(|_| LoopError::Terminate)
                        .and_then(move |opt_result|
                            match opt_result {
                                Some(result) => Ok(result),
                                None => Err(LoopError::Recoverable),
                            })
                })
                .and_then(|result| {
                    sink.send(result)
                        .map_err(|_| LoopError::Terminate)
                });

            match reruslt {
                Err(LoopError::Terminate) => break,
                Err(LoopError::Recoverable) => {}
                _ => {}
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
