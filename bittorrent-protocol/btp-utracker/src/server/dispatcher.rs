use std::io::{self, Cursor};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self};

use crate::announce::AnnounceRequest;
use crate::error::ErrorResponse;
use crate::request::{RequestType, TrackerRequest};
use crate::response::{ResponseType, TrackerResponse};
use crate::scrape::ScrapeRequest;
use crate::ServerHandler;
use nom::IResult;
use tokio::net::UdpSocket;
use tokio::{select, task};
use tokio::sync::mpsc;
// use umio::external::Sender;
// use umio::{Dispatcher, ELoopBuilder, Provider};
use crate::socket::UtSocket;

const EXPECTED_PACKET_LENGTH: usize = 1500;

/// Internal dispatch message for servers.
#[derive(Debug)]
pub enum DispatchMessage {
    Shutdown,
}

/// Create a new background dispatcher to service requests.
pub async fn create_dispatcher<H>(bind: SocketAddr, handler: H) -> io::Result<mpsc::UnboundedSender<DispatchMessage>>
where
    H: ServerHandler + 'static,
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

    let (command_tx, command_rx) = mpsc::unbounded_channel::<DispatchMessage>();
    let mut dispatch = ServerDispatcher::new(handler,command_rx,socket_arc,message_tx);

    task::spawn(dispatch.run_task());

    Ok(command_tx)
}

//----------------------------------------------------------------------------//

/// Dispatcher that executes requests asynchronously.
struct ServerDispatcher<H>
where
    H: ServerHandler,
{
    handler: H,
    is_run: bool,
    command_in: mpsc::UnboundedReceiver<DispatchMessage>,
    incoming: Arc<UtSocket>,
    message_send:  mpsc::UnboundedSender<(Vec<u8>, SocketAddr)>,
}

impl<H> ServerDispatcher<H>
where
    H: ServerHandler,
{
    /// Create a new ServerDispatcher.
    fn new(handler: H,
           cmd_rx: mpsc::UnboundedReceiver<DispatchMessage>,
           incom: Arc<UtSocket>,
           msg_send: mpsc::UnboundedSender<(Vec<u8>,SocketAddr)>) -> ServerDispatcher<H> {
        ServerDispatcher {
            handler: handler ,
            is_run: true,
            incoming:incom,
            message_send: msg_send,
            command_in: cmd_rx,
        }
    }


    pub(crate) async fn run_task(mut self){
        while  self.is_run {
            self.run_one().await;
        }

    }

    async fn run_one(&mut self){
        select! {

            msg  = self.incoming.recv() => {
                if let Ok((message,addr)) = msg {
                    self.incoming_in_handler(&message[..],addr).await
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
        }
    }

    async fn incoming_in_handler(&mut self, message : &[u8], addr: SocketAddr){
        let request = match TrackerRequest::from_bytes(message) {
            IResult::Done(_, req) => req,
            _ => return, // TODO: Add Logging
        };

        self.process_request(request, addr);
    }

    async fn command_in_handler(&mut self, message : DispatchMessage){
        match message {
          DispatchMessage::Shutdown => self.shutdown(),
        }
    }


    fn shutdown(&mut self){
        self.is_run = false;
    }
    /// Forward the request on to the appropriate handler method.
    fn process_request(
        &mut self,
        request: TrackerRequest,
        addr: SocketAddr,
    ) {
        let conn_id = request.connection_id();
        let trans_id = request.transaction_id();

        match request.request_type() {
            &RequestType::Connect => {
                if conn_id == crate::request::CONNECT_ID_PROTOCOL_ID {
                    self.forward_connect(trans_id, addr);
                } // TODO: Add Logging
            }
            &RequestType::Announce(ref req) => {
                self.forward_announce(trans_id, conn_id, req, addr);
            }
            &RequestType::Scrape(ref req) => {
                self.forward_scrape(trans_id, conn_id, req, addr);
            }
        };
    }

    /// Forward a connect request on to the appropriate handler method.
    fn forward_connect(
        &mut self,
        trans_id: u32,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();
        self.handler.connect(addr, |result| {
            let response_type = match result {
                Ok(conn_id) => ResponseType::Connect(conn_id),
                Err(err_msg) => ResponseType::Error(ErrorResponse::new(err_msg)),
            };
            let response = TrackerResponse::new(trans_id, response_type);

            write_response(response, addr,msg_send);
        });
    }

    /// Forward an announce request on to the appropriate handler method.
    fn forward_announce<'b>(
        &mut self,
        // provider: &mut Provider<'a, ServerDispatcher<H>>,
        trans_id: u32,
        conn_id: u64,
        request: &AnnounceRequest<'b>,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();

        self.handler.announce(addr, conn_id, request, |result| {
            let response_type = match result {
                Ok(response) => ResponseType::Announce(response),
                Err(err_msg) => ResponseType::Error(ErrorResponse::new(err_msg)),
            };
            let response = TrackerResponse::new(trans_id, response_type);

            write_response(response, addr,msg_send);
        });
    }

    /// Forward a scrape request on to the appropriate handler method.
    fn forward_scrape<'b>(
        &mut self,
        // provider: &mut Provider<'a, ServerDispatcher<H>>,
        trans_id: u32,
        conn_id: u64,
        request: &ScrapeRequest<'b>,
        addr: SocketAddr,
    ) {
        let msg_send = self.message_send.clone();

        self.handler.scrape(addr, conn_id, request, |result| {
            let response_type = match result {
                Ok(response) => ResponseType::Scrape(response),
                Err(err_msg) => ResponseType::Error(ErrorResponse::new(err_msg)),
            };
            let response = TrackerResponse::new(trans_id, response_type);

            write_response(response, addr,msg_send);
        });
    }

}

/// Write the given tracker response through to the given provider.
fn write_response(
    response: TrackerResponse,
    addr: SocketAddr,
    msg_send: mpsc::UnboundedSender<(Vec<u8>, SocketAddr)>,
) {
    let mut buffer= vec![0_u8;1500];
    let mut writer = Cursor::new(&mut buffer);
    let write_success = response.write_bytes(&mut writer).is_ok();

    if write_success {
        let posi = writer.position() as usize;
        msg_send
            .send((buffer[0..posi].to_vec(), addr))
            .expect("msg send fail");
    }

}

