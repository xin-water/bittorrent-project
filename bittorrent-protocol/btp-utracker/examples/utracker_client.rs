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
    let mut client = TrackerClient::new("0.0.0.0:4501".parse().unwrap(),[0_u8;20].into(),6969)
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
}