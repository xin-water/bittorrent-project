use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::{self, Cursor};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self};

use chrono::{DateTime, Duration, Utc};
use nom::IResult;
use rand::{self};
use futures::StreamExt;
use tokio::{select, sync::mpsc, task};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedSender;
use btp_util::bt::InfoHash;
// use umio::external::{self, Timeout};
// use umio::{Dispatcher, ELoopBuilder, Provider};
use btp_util::timer::{Timeout, Timer};

use crate::announce::{AnnounceRequest, DesiredPeers, SourceIP};
use crate::client::RequestLimiter;
use crate::option::AnnounceOptions;
use crate::request::{RequestType, TrackerRequest};
use crate::response::{ResponseType, TrackerResponse};
use crate::scrape::ScrapeRequest;
use crate::{
    request, ClientError, ClientMetadata, ClientRequest, ClientResponse, ClientResult, ClientToken
};
use crate::socket::UtSocket;

const EXPECTED_PACKET_LENGTH: usize = 1500;

const CONNECTION_ID_VALID_DURATION_MILLIS: i64 = 60000;
const MAXIMUM_REQUEST_RETRANSMIT_ATTEMPTS: u64 = 8;

/// Internal dispatch timeout.
enum DispatchTimeout {
    Connect(ClientToken),
    CleanUp,
}

/// Internal dispatch message for clients.
#[derive(Debug)]
pub enum DispatchMessage {
    Request(SocketAddr, ClientToken, ClientRequest,mpsc::UnboundedSender<ClientMetadata>),
    StartTimer,
    Shutdown,
}

/// Create a new background dispatcher to execute request and send responses back.
///
/// Assumes msg_capacity is less than usize::max_value().
pub async fn create_dispatcher(
    bind: SocketAddr,
    peer_id: InfoHash,
    Announce_port: u16,
    msg_capacity: usize,
    limiter: RequestLimiter,
) -> io::Result<mpsc::Sender<DispatchMessage>>
{

    let udp_socket = UdpSocket::bind(bind).await?;
    let socket_arc = Arc::new(UtSocket::new(udp_socket).unwrap());

    // 专门用一个协程发送数据，避免 数据处理中心 阻塞。
    let send_socket =socket_arc.clone();
    let (message_tx, mut message_rx)=mpsc::unbounded_channel::<(Vec<u8>, SocketAddr)>();
    //专门用一个协程发送数据，避免 数据处理中心 阻塞。
    task::spawn(async move {
        log::info!("消息发送协程已启动");
        while let Some((buffer,addr)) = message_rx.recv().await{
            send_socket.send(&buffer,addr).await;
        }
        log::warn!("消息发送协程已退出");

    });

    let (command_tx, command_rx) = mpsc::channel::<DispatchMessage>(100);
    let dispatch = ClientDispatcher::new(bind,peer_id,Announce_port, limiter,socket_arc,command_rx,message_tx);


    task::spawn(dispatch.run_task());

    command_tx
        .send(DispatchMessage::StartTimer)
        .await
        .expect("bittorrent-protocol_utracker: ELoop Failed To Start Connect ID Timer...");

    Ok(command_tx)
}

//----------------------------------------------------------------------------//

/// Dispatcher that executes requests asynchronously.
struct ClientDispatcher {
    bound_addr: SocketAddr,
    peer_id: InfoHash,
    Announce_port: u16,
    active_requests: HashMap<ClientToken, ConnectTimer>,
    id_cache: ConnectIdCache,
    limiter: RequestLimiter,
    is_run: bool,
    response_in: Arc<UtSocket>,
    command_in: mpsc::Receiver<DispatchMessage>,
    message_send:  mpsc::UnboundedSender<(Vec<u8>, SocketAddr)>,
    timer: Timer<DispatchTimeout>,
}

impl ClientDispatcher
{
    /// Create a new ClientDispatcher.
    pub fn new(
               bind: SocketAddr,
               peer_id: InfoHash,
               Announce_port:u16,
               limiter: RequestLimiter,
               response_in: Arc<UtSocket>,
               command_in: mpsc::Receiver<DispatchMessage>,
               message_send: mpsc::UnboundedSender<(Vec<u8>,SocketAddr)>,) -> ClientDispatcher {
        ClientDispatcher {
            bound_addr: bind,
            peer_id: peer_id,
            Announce_port: Announce_port,
            active_requests: HashMap::new(),
            id_cache: ConnectIdCache::new(),
            limiter: limiter,
            is_run: true,
            command_in,
            response_in,
            message_send: message_send,
            timer: Timer::new(),
        }
    }


    pub(crate) async fn run_task(mut self){
            while  self.is_run {
                self.run_one().await;
            }

    }


    async fn run_one(&mut self){
        select! {

            msg  = self.response_in.recv() => {
                if let Ok((message,addr)) = msg {
                    self.response_in_handler(&message,addr).await
                } else {
                    self.shutdown()
                }
           }

           cmd  = self.command_in.recv() => {
                if let Some(command) = cmd {
                    self.command_in_handler(command).await
                } else {
                    self.shutdown()
                }
           }

           token = self.timer.next(), if !self.timer.is_empty() => {
                let token = token.unwrap();
                self.timeout_handler(token).await
           }
        }
    }

    async fn response_in_handler(&mut self, message : &Vec<u8>, addr: SocketAddr){
        let response = match TrackerResponse::from_bytes(message) {
            IResult::Done(_, rsp) => rsp,
            _ => return, // TODO: Add Logging
        };

        self.recv_response(addr, response);
    }

    async fn command_in_handler(&mut self, message : DispatchMessage){
        match message {
            DispatchMessage::Request(addr, token, req_type, tx) => {
                self.send_request(addr, token, req_type, tx);
            }
            DispatchMessage::StartTimer => self.timeout_handler(DispatchTimeout::CleanUp).await,
            DispatchMessage::Shutdown => self.shutdown(),
        }
    }

    async fn timeout_handler(&mut self, timeout : DispatchTimeout){
        match timeout {
            DispatchTimeout::Connect(token) => self.process_request(token, true),
            DispatchTimeout::CleanUp => {
                self.id_cache.clean_expired();
                self.timer.schedule_in(std::time::Duration::from_secs(CONNECTION_ID_VALID_DURATION_MILLIS as u64 ),DispatchTimeout::CleanUp);
            }
        };
    }

    /// Shutdown the current dispatcher, notifying all pending requests.
    pub fn shutdown<'a>(&mut self) {
        // Notify all active requests with the appropriate error

        // for token_index in 0..self.active_requests.len() {
        //     let next_token = *self
        //         .active_requests
        //         //.keys()
        //         .skip(token_index)
        //         .next()
        //         .unwrap();
        //
        //     self.notify_client(next_token, Err(ClientError::ClientShutdown));
        // }

        for (token,timeout) in self.active_requests.iter() {

            timeout.tx.send(ClientMetadata::new(*token,Err(ClientError::ClientShutdown))).expect("ClientShutdown send fail");
        }
        // TODO: Clear active timeouts
        self.active_requests.clear();

        self.is_run = false;
    }

    /// Finish a request by sending the result back to the client.
    // pub fn notify_client(&mut self, token: ClientToken, result: ClientResult<ClientResponse>) {
    //     self.handshaker
    //         .metadata(ClientMetadata::new(token, result).into());
    //
    //     self.limiter.acknowledge();
    // }

    /// Process a request to be sent to the given address and associated with the given token.
    pub fn send_request<'a>(
        &mut self,
        addr: SocketAddr,
        token: ClientToken,
        request: ClientRequest,
        tx:UnboundedSender<ClientMetadata>
    ) {
        // Check for IP version mismatch between source addr and dest addr
        match (self.bound_addr, addr) {
            (SocketAddr::V4(_), SocketAddr::V6(_)) | (SocketAddr::V6(_), SocketAddr::V4(_)) => {
                //self.notify_client(token, Err(ClientError::IPVersionMismatch));
                tx.send(ClientMetadata::new(
                    token,
                    Err(ClientError::IPVersionMismatch)
                )).expect("Err IPVersionMismatch send fail");
                return;
            }
            _ => (),
        };
        self.active_requests
            .insert(token, ConnectTimer::new(addr, request, tx));

        self.process_request(token, false);
    }

    /// Process a response received from some tracker and match it up against our sent requests.
    pub fn recv_response(
        &mut self,
        addr: SocketAddr,
        response: TrackerResponse,
    ) {
        let token: ClientToken = ClientToken {
            token: response.transaction_id(),
        };

        let conn_timer = if let Some(conn_timer) = self.active_requests.remove(&token) {
            let socket_addr = conn_timer.message_params().0;
            if socket_addr == addr {
                conn_timer
            } else {
                return;
            } // TODO: Add Logging (Server Receive Addr Different Than Send Addr)
        } else {
            return;
        }; // TODO: Add Logging (Server Gave Us Invalid Transaction Id)


        self.timer.cancel(conn_timer.timeout_id.unwrap());


        // Check if the response requires us to update the connection timer
        if let &ResponseType::Connect(id) = response.response_type() {
            self.id_cache.put(addr, id);

            self.active_requests.insert(token, conn_timer);
            self.process_request(token, false);
        } else {
            // Match the request type against the response type and update our client
            match (conn_timer.message_params().1, response.response_type()) {
                (&ClientRequest::Announce(hash, _), &ResponseType::Announce(ref res)) => {
                    // Forward contact information on to the handshaker
                    // for addr in res.peers().iter() {
                    //     self.handshaker.connect(None, hash, addr);
                    // }

                    // self.notify_client(token, Ok(ClientResponse::Announce(res.to_owned())));
                    conn_timer.tx.send(ClientMetadata::new(token,Ok(ClientResponse::Announce(res.to_owned())))).expect("Announce send fail");
                }
                (&ClientRequest::Scrape(..), &ResponseType::Scrape(ref res)) => {
                   // self.notify_client(token, Ok(ClientResponse::Scrape(res.to_owned())));
                    conn_timer.tx.send(ClientMetadata::new(token,Ok(ClientResponse::Scrape(res.to_owned())))).expect("Scrape send fail");

                }
                (_, &ResponseType::Error(ref res)) => {
                    //self.notify_client(token, Err(ClientError::ServerMessage(res.to_owned())));
                    conn_timer.tx.send(ClientMetadata::new(token,Err(ClientError::ServerMessage(res.to_owned())))).expect("ServerMessage send fail");
                }
                _ => {
                    //self.notify_client(token, Err(ClientError::ServerError));
                    conn_timer.tx.send(ClientMetadata::new(token,Err(ClientError::ServerError))).expect("ServerError send fail");

                }
            }
        }
    }

    /// Process an existing request, either re requesting a connection id or sending the actual request again.
    ///
    /// If this call is the result of a timeout, that will decide whether to cancel the request or not.
    fn process_request<'a>(
        &mut self,
        token: ClientToken,
        timed_out: bool,
    ) {
        let mut conn_timer = if let Some(conn_timer) = self.active_requests.remove(&token) {
            conn_timer
        } else {
            return;
        }; // TODO: Add logging

        // Resolve the duration of the current timeout to use
        let next_timeout = match conn_timer.current_timeout(timed_out) {
            Some(timeout) => timeout,
            None => {
                //self.notify_client(token, Err(ClientError::MaxTimeout));
                conn_timer.tx.send(ClientMetadata::new(token, Err(ClientError::MaxTimeout))).expect("MaxTimeout send fail");

                return;
            }
        };

        let addr = conn_timer.message_params().0;
        let opt_conn_id = self.id_cache.get(conn_timer.message_params().0);

        // Resolve the type of request we need to make
        let (conn_id, request_type) = match (opt_conn_id, conn_timer.message_params().1) {
            (Some(id), &ClientRequest::Announce(hash, state)) => {
                let source_ip = match addr {
                    SocketAddr::V4(_) => SourceIP::ImpliedV4,
                    SocketAddr::V6(_) => SourceIP::ImpliedV6,
                };
                let key = rand::random::<u32>();

                (
                    id,
                    RequestType::Announce(AnnounceRequest::new(
                        hash,
                        self.peer_id,
                        state,
                        source_ip,
                        key,
                        DesiredPeers::Default,
                        self.Announce_port,
                        AnnounceOptions::new(),
                    )),
                )
            }
            (Some(id), &ClientRequest::Scrape(hash)) => {
                let mut scrape_request = ScrapeRequest::new();
                scrape_request.insert(hash);

                (id, RequestType::Scrape(scrape_request))
            }
            (None, _) => (request::CONNECT_ID_PROTOCOL_ID, RequestType::Connect),
        };
        let tracker_request = TrackerRequest::new(conn_id, token.token, request_type);


        // Try to write the request out to the server

        let mut write_success = false;

        {
            let mut buffer= vec![0_u8;1500];
            let mut writer = Cursor::new(&mut buffer);
            write_success = tracker_request.write_bytes(&mut writer).is_ok();

            if write_success {
                 let posi = writer.position() as usize;
                self.message_send.send((buffer[0..posi].to_vec(), addr)).expect("msg send fail");

            }
        }


        // If message was not sent (too long to fit) then end the request
        if !write_success {
            //self.notify_client(token, Err(ClientError::MaxLength));
            conn_timer.tx.send(ClientMetadata::new(token,Err(ClientError::MaxLength))).expect("MaxLength send fail");
        } else {

            conn_timer.set_timeout_id(
                self.timer.schedule_in(std::time::Duration::from_secs(next_timeout ),DispatchTimeout::Connect(token))
            );

            self.active_requests.insert(token, conn_timer);
        }
    }
}

//----------------------------------------------------------------------------//

/// Contains logic for making sure a valid connection id is present
/// and correctly timing out when sending requests to the server.
struct ConnectTimer {
    addr: SocketAddr,
    attempt: u64,
    request: ClientRequest,
    timeout_id: Option<Timeout>,
    tx: UnboundedSender<ClientMetadata>,
}

impl ConnectTimer {
    /// Create a new ConnectTimer.
    pub fn new(addr: SocketAddr, request: ClientRequest,tx: UnboundedSender<ClientMetadata>) -> ConnectTimer {
        ConnectTimer {
            addr: addr,
            attempt: 0,
            request: request,
            timeout_id: None,
            tx:tx,
        }
    }

    /// Yields the current timeout value to use or None if the request should time out completely.
    pub fn current_timeout(&mut self, timed_out: bool) -> Option<u64> {
        if self.attempt == MAXIMUM_REQUEST_RETRANSMIT_ATTEMPTS {
            None
        } else {
            if timed_out {
                self.attempt += 1;
            }

            Some(calculate_message_timeout_millis(self.attempt))
        }
    }

    /// Yields the current timeout id if one is set.
    pub fn timeout_id(&self) -> Option<Timeout> {
        self.timeout_id
    }

    /// Sets a new timeout id.
    pub fn set_timeout_id(&mut self, id: Timeout) {
        self.timeout_id = Some(id);
    }

    /// Yields the message parameters for the current connection.
    pub fn message_params(&self) -> (SocketAddr, &ClientRequest) {
        (self.addr, &self.request)
    }
}

/// Calculates the timeout for the request given the attempt count.
fn calculate_message_timeout_millis(attempt: u64) -> u64 {
    (15 * 2u64.pow(attempt as u32)) * 1000
}

//----------------------------------------------------------------------------//

/// Cache for storing connection ids associated with a specific server address.
struct ConnectIdCache {
    cache: HashMap<SocketAddr, (u64, DateTime<Utc>)>,
}

impl ConnectIdCache {
    /// Create a new connect id cache.
    fn new() -> ConnectIdCache {
        ConnectIdCache {
            cache: HashMap::new(),
        }
    }

    /// Get an un expired connection id for the given addr.
    fn get(&mut self, addr: SocketAddr) -> Option<u64> {
        match self.cache.entry(addr) {
            Entry::Vacant(_) => None,
            Entry::Occupied(occ) => {
                let curr_time = Utc::now();
                let prev_time = occ.get().1;

                if is_expired(curr_time, prev_time) {
                    occ.remove();

                    None
                } else {
                    Some(occ.get().0)
                }
            }
        }
    }

    /// Put an un expired connection id into cache for the given addr.
    fn put(&mut self, addr: SocketAddr, connect_id: u64) {
        let curr_time = Utc::now();

        self.cache.insert(addr, (connect_id, curr_time));
    }

    /// Removes all entries that have expired.
    fn clean_expired(&mut self) {
        let curr_time = Utc::now();
        let mut curr_index = 0;

        let mut opt_curr_entry = self
            .cache
            .iter()
            .skip(curr_index)
            .map(|(&k, &v)| (k, v))
            .next();
        while let Some((addr, (_, prev_time))) = opt_curr_entry.take() {
            if is_expired(curr_time, prev_time) {
                self.cache.remove(&addr);
            }

            curr_index += 1;
            opt_curr_entry = self
                .cache
                .iter()
                .skip(curr_index)
                .map(|(&k, &v)| (k, v))
                .next();
        }
    }
}

/// Returns true if the connect id received at prev_time is now expired.
fn is_expired(curr_time: DateTime<Utc>, prev_time: DateTime<Utc>) -> bool {
    let valid_duration = Duration::milliseconds(CONNECTION_ID_VALID_DURATION_MILLIS);

    curr_time - prev_time >= valid_duration
}
