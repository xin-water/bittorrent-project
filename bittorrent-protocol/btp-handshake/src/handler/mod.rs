use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use futures::{Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::{select, task};
use tokio::sync::mpsc;
use crate::filter::filters::Filters;
use crate::{CompleteMessage, Extensions, FilterDecision, InitiateMessage, LocalAddr, Protocol, Transport};
use btp_util::bt::{InfoHash, PeerId};

pub mod handshaker;

pub(crate) async fn srart_handler_task<S,T,L>(
    pid: PeerId,
    ext: Extensions,
    transport: T,
    listener:L,
    command:mpsc::Receiver<InitiateMessage>,
    sock_send:mpsc::UnboundedSender<CompleteMessage<S>>,
    filters:Filters,
)
where S: AsyncRead + AsyncWrite + Send + Unpin +'static + Debug,
      T: Transport<Socket = S> + Send + Sync + 'static,
      L: Stream<Item=(S,SocketAddr)> + Send + Unpin  + 'static,
{
    let mut handler = HandshakeHandler::new(pid,ext,transport,listener,command,sock_send,filters);
    task::spawn(handler.run());
}

pub struct HandshakeHandler<S,T,L>{
    is_run: bool,
    filters: Arc<Filters>,
    ext: Extensions,
    pid: PeerId,
    command_rx: mpsc::Receiver<InitiateMessage>,
    transport: T,
    listen: L,
    out_msg: mpsc::UnboundedSender<CompleteMessage<S>>
}

impl<S,T,L> HandshakeHandler<S,T,L>
where S: AsyncRead + AsyncWrite + Send + Unpin + Debug + 'static,
      L: Stream<Item=(S, SocketAddr)> + Send + Unpin,
      T: Transport<Socket = S> + Send + std::marker::Sync,
{

    fn new( pid: PeerId,ext: Extensions,transport: T, listener:L, command:mpsc::Receiver<InitiateMessage>, sock_send:mpsc::UnboundedSender<CompleteMessage<S>>, filters:Filters,) ->Self{

        HandshakeHandler{
            is_run: true,
            ext: ext,
            pid: pid,
            filters: Arc::new(filters),
            command_rx: command,
            transport: transport,
            listen: listener,
            out_msg: sock_send
        }
    }

    fn shutdown(&mut self){
        self.is_run = false;
    }

    pub async fn run(mut self){
        while self.is_run {
            self.run_one().await
        }
    }

    pub async fn run_one(&mut self){
        select! {
            command = self.command_rx.recv() => {
                if let Some(command) = command {
                    self.handle_command(command).await
                } else {
                    self.shutdown()
                }
            }
            message = self.listen.next() => {
                match message {
                    Some((socket, addr)) =>  self.handle_listen(socket, addr).await,
                    None => log::warn!("Failed to receive listen message"),
                }
            }
        }
    }


    pub(crate) async fn handle_command(&mut self, item:InitiateMessage){

        // 过虑判断，不应该过虑就发起链接
        if !should_filter(Some(item.address()),
                          Some(item.protocol()),
                          None,
                          Some(item.hash()),
                          None,
                          &self.filters
        ){
            let res_connect = self.transport.connect(item.address()).await;
            if let Ok(socket) = res_connect {
                match handshaker::initiate_handshake(socket,item,self.ext.clone(),self.pid.clone(),self.filters.clone()).await {
                    Ok(Some(s))=>self.out_msg.send(s).expect("send initiate_handshake msg fail"),
                    _ => {}
                }
            }
        }
    }

    pub(crate) async fn handle_listen(&mut self, socket:S, addr:SocketAddr){

        if !should_filter(Some(&addr), None, None, None, None, &self.filters.clone()) {
           match handshaker::complete_handshake(socket,addr,self.ext.clone(),self.pid.clone(),self.filters.clone()).await{
               Ok(Some(s))=>self.out_msg.send(s).expect("send complete_handshake msg fail"),
               _ => {}
           }
        }
    }

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
