use std::net::SocketAddr;
use std::sync::{Arc, mpsc, Mutex};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use btp_util::bt;
use btp_util::bt::{InfoHash, PeerId};
use btp_utracker::{ClientMetadata, ClientRequest, Handshaker, TrackerClient, TrackerServer};
use btp_utracker::announce::{AnnounceEvent, ClientState};

#[tokio::main]
async fn main(){

    // 启动tracker client
    let (send, recv) = mpsc::channel();
    let mock_handshaker = MockHandshaker::new(send);
    let mut client = TrackerClient::new("0.0.0.0:4501".parse().unwrap(), mock_handshaker.clone())
            .await
            .unwrap();

    // 发起注册请求
    // utracker_server  "udp://bt.ktrackers.com:6666/announce"
    // ping bt.ktrackers.com   ->  195.154.237.90
    let server_addr: SocketAddr = "195.154.237.90:6666".parse().unwrap();
    let hash = InfoHash::from_hex("3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0");
    let mut rx = client
        .request(
            server_addr,
            ClientRequest::Announce(
                hash,
                ClientState::new(0, 0, 0, AnnounceEvent::Started),
            ))
        .await
        .unwrap();

    // 响应输出
    //let metadata = recv.recv().unwrap();
    let metadata = rx.recv().await.unwrap();

    println!("metadata:{:?}",metadata.token());

    let response = metadata
        .result()
        .as_ref()
        .unwrap()
        .announce_response()
        .unwrap();
    println!("response leechers:{:?}",response.leechers());
    println!("response peers:{:?}",response.peers().iter().count());
    for peer in response.peers().iter() {
        println!("response socket:{:?}",peer);
    }

    mock_handshaker.connects_received(|connects| {
        println!("connects len:{:?}",connects.len());
    });


}







#[derive(Clone)]
struct MockHandshaker {
    send: Sender<ClientMetadata>,
    connects: Arc<Mutex<Vec<SocketAddr>>>,
}

impl MockHandshaker {
    pub fn new(send: Sender<ClientMetadata>) -> MockHandshaker {
        MockHandshaker {
            send: send,
            connects: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn connects_received<F>(&self, callback: F)
        where
            F: FnOnce(&[SocketAddr]),
    {
        let locked = self.connects.lock().unwrap();

        callback(&*locked);
    }
}

impl Handshaker for MockHandshaker {
    type Metadata = ClientMetadata;

    fn id(&self) -> PeerId {
        [0u8; 20].into()
    }

    fn port(&self) -> u16 {
        6969
    }

    fn connect(&mut self, _: Option<PeerId>, _: InfoHash, addr: SocketAddr) {
        self.connects.lock().unwrap().push(addr);
    }

    fn metadata(&mut self, data: ClientMetadata) {
        self.send.send(data).unwrap();
    }
}