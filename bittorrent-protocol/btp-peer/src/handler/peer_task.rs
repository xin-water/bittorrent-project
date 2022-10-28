#![allow(deprecated)]

use super::peer_info::PeerInfo;
use super::{IPeerManagerMessage, OPeerManagerMessage};
use crate::message::PeerWireProtocolMessage;
use bytes::Bytes;
use std::net::TcpStream;
use std::io::{Read, Cursor, Write};
use crate::{PeerWireMessageCodec, MessageCodec};
use std::sync::{Arc, Mutex};
use std::borrow::BorrowMut;
use std::future::Future;
use tokio::io::{AsyncRead,AsyncReadExt, AsyncWrite,AsyncWriteExt};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task;
use crate::split::Split;

pub async fn run_peer_task<S>(
    peer_stream: S,
    info: PeerInfo,
    mut msg_rx: UnboundedReceiver<IPeerManagerMessage<S>>,
    o_send: UnboundedSender<OPeerManagerMessage>,
)
    where S: AsyncRead + AsyncWrite + Split + Send + 'static,
{

    let  (mut peer_read,peer_write) = peer_stream.split();
    let msg_codec = Arc::new(Mutex::new(PeerWireMessageCodec::new()));

    // 启动数据读取
    let o_send1 = o_send.clone();
    let me_info = info.clone();
    let me_msg_codec = msg_codec.clone();
    task::spawn(loop_read_msg(peer_read, o_send1, me_info, me_msg_codec));

    // 启动数据写入
    task::spawn(loop_write_msg(info, msg_rx, o_send, peer_write, msg_codec));
}


async fn loop_read_msg<R>(mut peer_read: R,
                       o_send1: UnboundedSender<OPeerManagerMessage>,
                       me_info: PeerInfo,
                       me_msg_codec: Arc<Mutex<PeerWireMessageCodec>>)
where R: AsyncRead + AsyncReadExt + Send + 'static + Unpin,
{
    let num = 24 * 1024;
    let mut in_buffer = Cursor::new(vec![0u8; num]);
    let mut read_position = 0;
    loop {

        // 读取数据到缓存区
        {
            read_position = in_buffer.position() as usize;
            let in_slice:&mut [u8] = &mut in_buffer.get_mut()[read_position..];
            let read_result = peer_read.read(in_slice).await;
            match read_result{
                Ok(bytes_read) => {
                    read_position += bytes_read;
                }
                Err(error) => {
                    //读取出错，可能流已经关闭了，直接返回，结束工作
                    log::error!("peer_read read data error:{:?}",error);
                    return;
                }
            }

            in_buffer.set_position(read_position as u64);
        }


        // Try to parse whatever part of the message we currently have (see if we need to disconnect early)
        let mut data_slice: &[u8] = &in_buffer.get_mut()[..read_position];

        loop {
            let me_msg_code_lock = me_msg_codec.lock();
            if let Ok(mut msg_codec) = me_msg_code_lock {
                log::trace!("[peer loop_read_msg] read read_position:{:?}",read_position);
                log::trace!("[peer loop_read_msg] msg_head:{:?}",&data_slice[0..4]);

                //此处使用 if let 则在接受到 多个数据时只会解析一个,造成卡顿.
                //此处使用 while let ,在输入缓冲大时可提高性能,但要处理数据不全时 数据头里记录的长度与读取到的长度不相符而导致的断言异常
                while let Ok(msg) = msg_codec.parse_bytes(Bytes::from(data_slice)) {
                    let message_size = msg.message_size();
                    info!("[peer loop_read_msg] message_size:{:?}\n",message_size);

                    data_slice = &data_slice[message_size..];
                    //data_slice= &(in_buffer.get_mut()[msg.message_size()..read_position].to_vec());

                    o_send1.send(OPeerManagerMessage::ReceivedMessage(me_info, msg)).unwrap();
                }

                let mut v = data_slice.to_vec();
                let len = v.len();
                if len < num {
                    v.append(vec![0_u8; num - len].borrow_mut());
                }

                in_buffer = Cursor::new(v);
                in_buffer.set_position(len as u64);

                break;
            }
        }
    }
}

async fn loop_write_msg<S,W>(info: PeerInfo,
                             mut msg_rx: UnboundedReceiver<IPeerManagerMessage<S>>,
                             o_send: UnboundedSender<OPeerManagerMessage>,
                             mut peer_write: W,
                             msg_codec: Arc<Mutex<PeerWireMessageCodec>>)
where
    S: AsyncRead + AsyncWrite + Split + Send + 'static,
    W: AsyncWrite + AsyncWriteExt + Send + 'static + Unpin,

{
    o_send.send(OPeerManagerMessage::PeerAdded(info)).unwrap();
    let num = 24 * 1024;
    let mut out_buffer = Cursor::new(vec![0u8; num]);

    loop {
        //构造result
         match msg_rx.recv().await {
            Some(IPeerManagerMessage::SendMessage(p_info, mid, p_message)) =>{
                log::trace!("loop_write_msg SendMessage:{:?}",p_message);
                out_buffer.set_position(0);

                //获取锁，消息序列化写入缓冲区
                loop {
                    let msg_codec_lock = msg_codec.lock();

                    if let Ok(mut msg_codec) = msg_codec_lock {
                        msg_codec
                            .write_bytes(&p_message,&mut out_buffer)
                            .unwrap();
                        break;
                    }
                }

                // 发送消息
                // 不能在上面循环里直接异步发送，
                // 锁范围内使用异步会造成重入锁
                peer_write.write(&out_buffer.get_ref()[..p_message.message_size()]).await;
                o_send.send(OPeerManagerMessage::SentMessage(p_info, mid)).expect(" send SentMessage fail");
            },

            Some(IPeerManagerMessage::RemovePeer(p_info)) => {
                o_send.send(OPeerManagerMessage::PeerRemoved(p_info)).expect(" send PeerRemoved fail");
                break;
            },

            None => {
                o_send.send(OPeerManagerMessage::PeerDisconnect(info)).expect(" send PeerDisconnect fail");
                break;
            },
            _ => {
                 log::error!("bittorrent-protocol_peer: Peer Future Received Invalid Message From Peer Manager");
                 break;
             },
        }
    }
}
