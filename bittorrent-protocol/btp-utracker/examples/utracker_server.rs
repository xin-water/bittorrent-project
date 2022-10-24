use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, mpsc, Mutex};
use std::sync::mpsc::Sender;
use std::thread;
use tokio::time::{sleep, Duration};
use btp_util::bt;
use btp_util::bt::{InfoHash, PeerId};
use btp_util::trans::old::TIDGenerator;
use btp_utracker::{ClientMetadata, ClientRequest, ServerHandler, ServerResult, TrackerClient, TrackerServer};
use btp_utracker::announce::{AnnounceEvent, AnnounceRequest, AnnounceResponse, ClientState};
use btp_utracker::contact::{CompactPeers, CompactPeersV4, CompactPeersV6};
use btp_utracker::scrape::{ScrapeRequest, ScrapeResponse, ScrapeStats};

#[tokio::main]
async fn main(){

     //启动 tracker server
    let server_addr = "0.0.0.0:3501".parse().unwrap();
    let server = TrackerServer::run(server_addr).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // 启动tracker client
    let mut client = TrackerClient::new("0.0.0.0:4501".parse().unwrap(),[0_u8;20].into(),6969)
        .await
        .unwrap();

    // 向tracker服务器发起注册请求
    let hash = InfoHash::from_hex("3b245504cf5f11bbdbe1201cea6a6bf45aee1bc0");

    //不能直接用 "0.0.0.0:3501" 作为目标地址。因为返回的响应地址是具体的。
    let server_addr = "127.0.0.1:3501".parse().unwrap();

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