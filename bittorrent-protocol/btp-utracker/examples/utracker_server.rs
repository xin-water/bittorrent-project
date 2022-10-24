use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::{Arc, mpsc, Mutex};
use std::sync::mpsc::Sender;
use std::thread;
use tokio::time::{sleep, Duration};
use btp_util::bt;
use btp_util::bt::{InfoHash, PeerId};
use btp_util::trans::old::TIDGenerator;
use btp_utracker::{ClientMetadata, ClientRequest, Handshaker, ServerHandler, ServerResult, TrackerClient, TrackerServer};
use btp_utracker::announce::{AnnounceEvent, AnnounceRequest, AnnounceResponse, ClientState};
use btp_utracker::contact::{CompactPeers, CompactPeersV4, CompactPeersV6};
use btp_utracker::scrape::{ScrapeRequest, ScrapeResponse, ScrapeStats};

#[tokio::main]
async fn main(){

     //启动 tracker server
    let server_addr = "0.0.0.0:3501".parse().unwrap();
    let mock_handler = MockTrackerHandler::new();
    let server = TrackerServer::run(server_addr, mock_handler).await.unwrap();

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

////////////////////////////////////////////////////////////////////////////

const NUM_PEERS_RETURNED: usize = 20;

#[derive(Clone)]
struct MockTrackerHandler {
    inner: Arc<Mutex<InnerMockTrackerHandler>>,
}

struct InnerMockTrackerHandler {
    cids: HashSet<u64>,
    cid_generator: TIDGenerator<u64>,
    peers_map: HashMap<InfoHash, HashSet<SocketAddr>>,
}

impl MockTrackerHandler {
    pub fn new() -> MockTrackerHandler {
        MockTrackerHandler {
            inner: Arc::new(Mutex::new(InnerMockTrackerHandler {
                cids: HashSet::new(),
                cid_generator: TIDGenerator::<u64>::new(),
                peers_map: HashMap::new(),
            })),
        }
    }

    pub fn num_active_connect_ids(&self) -> usize {
        self.inner.lock().unwrap().cids.len()
    }
}

impl ServerHandler for MockTrackerHandler {
    fn connect<R>(&mut self, _: SocketAddr, result: R)
        where
            R: for<'a> FnOnce(ServerResult<'a, u64>),
    {
        let mut inner_lock = self.inner.lock().unwrap();

        let cid = inner_lock.cid_generator.generate();
        inner_lock.cids.insert(cid);

        result(Ok(cid));
    }

    fn announce<'b, R>(&mut self, addr: SocketAddr, id: u64, req: &AnnounceRequest<'b>, result: R)
        where
            R: for<'a> FnOnce(ServerResult<'a, AnnounceResponse<'a>>),
    {
        let mut inner_lock = self.inner.lock().unwrap();

        if inner_lock.cids.contains(&id) {
            let peers = inner_lock
                .peers_map
                .entry(req.info_hash())
                .or_insert(HashSet::new());
            // Ignore any source ip directives in the request
            let store_addr = match addr {
                SocketAddr::V4(v4_addr) => {
                    SocketAddr::V4(SocketAddrV4::new(*v4_addr.ip(), req.port()))
                }
                SocketAddr::V6(v6_addr) => {
                    SocketAddr::V6(SocketAddrV6::new(*v6_addr.ip(), req.port(), 0, 0))
                }
            };

            // Resolve what to do with the event
            match req.state().event() {
                AnnounceEvent::None => peers.insert(store_addr),
                AnnounceEvent::Completed => peers.insert(store_addr),
                AnnounceEvent::Started => peers.insert(store_addr),
                AnnounceEvent::Stopped => peers.remove(&store_addr),
            };

            // Check what type of peers the request warrants
            let compact_peers = if req.source_ip().is_ipv4() {
                let mut v4_peers = CompactPeersV4::new();

                for v4_addr in peers
                    .iter()
                    .filter_map(|addr| match addr {
                        &SocketAddr::V4(v4_addr) => Some(v4_addr),
                        &SocketAddr::V6(_) => None,
                    })
                    .take(NUM_PEERS_RETURNED)
                {
                    v4_peers.insert(v4_addr);
                }

                CompactPeers::V4(v4_peers)
            } else {
                let mut v6_peers = CompactPeersV6::new();

                for v6_addr in peers
                    .iter()
                    .filter_map(|addr| match addr {
                        &SocketAddr::V4(_) => None,
                        &SocketAddr::V6(v6_addr) => Some(v6_addr),
                    })
                    .take(NUM_PEERS_RETURNED)
                {
                    v6_peers.insert(v6_addr);
                }

                CompactPeers::V6(v6_peers)
            };

            result(Ok(AnnounceResponse::new(
                1800,
                peers.len() as i32,
                peers.len() as i32,
                compact_peers,
            )));
        } else {
            result(Err("Connection ID Is Invalid"));
        }
    }

    fn scrape<'b, R>(&mut self, _: SocketAddr, id: u64, req: &ScrapeRequest<'b>, result: R)
        where
            R: for<'a> FnOnce(ServerResult<'a, ScrapeResponse<'a>>),
    {
        let mut inner_lock = self.inner.lock().unwrap();

        if inner_lock.cids.contains(&id) {
            let mut response = ScrapeResponse::new();

            for hash in req.iter() {
                let peers = inner_lock.peers_map.entry(hash).or_insert(HashSet::new());

                response.insert(ScrapeStats::new(peers.len() as i32, 0, peers.len() as i32));
            }

            result(Ok(response));
        } else {
            result(Err("Connection ID Is Invalid"));
        }
    }
}