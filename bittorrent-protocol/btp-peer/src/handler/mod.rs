use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender, UnboundedReceiver, UnboundedSender};
use tokio::{select, task};
use crate::{IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder};
use crate::split::Split;


mod peer_task;
pub mod peer_info;


pub(crate) fn start_peer_task<S>(build: PeerManagerBuilder)->(Sender<IPeerManagerMessage<S>>,UnboundedReceiver<OPeerManagerMessage>)
where S: AsyncWrite + AsyncRead + Send + 'static + Debug + Split,

{

    let (command_tx,command_rx) = mpsc::channel(build.sink_buffer_capacity());
    let (msg_tx,msg_rx) = mpsc::unbounded_channel();

    let handler= PeerHandler::new(build.peer_capacity(),command_rx,msg_tx);

    task::spawn(handler.run_task());

    return (command_tx,msg_rx);
}



pub(crate) struct PeerHandler<S>{
    is_run: bool,
    peers: Arc<Mutex<HashMap<PeerInfo, UnboundedSender<IPeerManagerMessage<S>>>>>,
    peer_capacity:usize,
    command: Receiver<IPeerManagerMessage<S>>,
    response_send: UnboundedSender<OPeerManagerMessage>,
    response_in: UnboundedReceiver<OPeerManagerMessage>,
    out_send: UnboundedSender<OPeerManagerMessage>,
}


impl<S> PeerHandler<S>
where S: AsyncRead + AsyncWrite + Send + 'static + Debug + Split,

{
    fn new(peer_capacity:usize,command:Receiver<IPeerManagerMessage<S>>,out_send: UnboundedSender<OPeerManagerMessage>)->Self{
        let (response_send,response_in)= mpsc::unbounded_channel();

        PeerHandler{
            is_run: true,
            peers: Arc::new(Mutex::new(HashMap::new())),
            peer_capacity: peer_capacity,
            command:command,
            out_send:out_send,
            response_send: response_send,
            response_in: response_in,
        }

    }


    fn shutdown(&mut self){
        self.is_run = false;
    }

    pub(crate) async fn run_task(mut self){

        while  self.is_run {
            self.run_one().await;
        }

    }

    async fn run_one(&mut self){
        select! {

            command  = self.command.recv() => {
                if let Some(cmd) = command {
                    self.command_ex(cmd).await
                } else {
                    self.shutdown()
                }
           }

            msg  = self.response_in.recv() => {
                if let Some(message) = msg {
                    self.response_in_ex(message).await
                } else {
                    self.shutdown()
                }
           }
        }
    }

    async fn command_ex(&mut self, message: IPeerManagerMessage<S>){

        match message {
            IPeerManagerMessage::AddPeer(info, peer) => {

                if let Ok(mut peers) = self.peers.try_lock() {

                    if peers.len() >= self.peer_capacity {
                        panic!("bittorrent-protocol_peer: PeerManager Failed To Send AddPeer");
                    } else {
                        match peers.entry(info) {
                            Entry::Occupied(_) => panic!(
                                "bittorrent-protocol_peer: PeerManager Failed To Send AddPeer"
                            ),
                            Entry::Vacant(vac) => {
                                let (msg_send,msg_rx) = mpsc::unbounded_channel();
                                vac.insert(msg_send);

                                task::spawn(peer_task::run_peer_task(peer, info, msg_rx, self.response_send.clone()));
                            }
                        }
                    }

                }
            }
            IPeerManagerMessage::RemovePeer(info) => {

                if let Ok(mut peers) = self.peers.try_lock() {

                    peers
                        .get(&info)
                        .unwrap()
                        .send(IPeerManagerMessage::RemovePeer(info))
                        .expect("bittorrent-protocol_peer: PeerManager Failed To Send RemovePeer");

                }
            }
            IPeerManagerMessage::SendMessage(info, mid, peer_message) => {

                if let Ok(mut peers) = self.peers.try_lock() {

                    peers
                        .get(&info)
                        .unwrap()
                        .send(IPeerManagerMessage::SendMessage(info, mid, peer_message))
                        .expect("bittorrent-protocol_peer: PeerManager Failed to Send SendMessage");

                }

            }
        }

    }

    async fn response_in_ex(&mut self, message: OPeerManagerMessage){

        match message{
            OPeerManagerMessage::PeerRemoved(info) => {
                if let Ok(mut peers) = self.peers.try_lock() {

                    peers
                        .remove(&info)
                        .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerRemoved Message With No Matching Peer In Map"));

                    self.out_send
                        .send(OPeerManagerMessage::PeerRemoved(info))
                        .expect(" out msg fail ");

                }

            },

            OPeerManagerMessage::PeerDisconnect(info) =>{
                if let Ok(mut peers) = self.peers.try_lock() {

                    peers
                        .remove(&info)
                        .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerDisconnect Message With No Matching Peer In Map"));

                    self.out_send
                        .send(OPeerManagerMessage::PeerDisconnect(info))
                        .expect(" out msg fail ");;

                }


            },

            OPeerManagerMessage::PeerError(info, error) =>{

                if let Ok(mut peers) = self.peers.try_lock() {

                    peers
                        .remove(&info)
                        .unwrap_or_else(|| panic!("bittorrent-protocol_peer: Received PeerError Message With No Matching Peer In Map"));



                    self.out_send
                        .send(OPeerManagerMessage::PeerError(info, error))
                        .expect(" out msg fail ");

                }

            },

            other =>{
                self.out_send
                    .send(other)
                    .expect(" out msg fail ");
            },
        }

    }

}
