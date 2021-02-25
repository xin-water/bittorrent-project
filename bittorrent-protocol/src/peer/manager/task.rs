#![allow(deprecated)]

use super::peer_info::PeerInfo;
use super::{IPeerManagerMessage, OPeerManagerMessage};
use crate::peer::message::PeerWireProtocolMessage;
use bytes::Bytes;
use std::net::TcpStream;
use std::io::{Read, Cursor, Write};
use std::sync::mpsc::{self, Sender};
use crate::peer::{PeerWireMessageCodec, MessageCodec};
use std::sync::{Arc, Mutex};
use std::borrow::BorrowMut;
use crate::peer::manager::TryClone;

pub fn run_peer<S>(
    peer: S,
    info: PeerInfo,
    o_send: Sender<OPeerManagerMessage>,
) -> Sender<IPeerManagerMessage>
    where S: Read + Write + TryClone + Send + 'static,
          <S as TryClone>::Item: Send {

    let mut p_recv = peer.try_clone().unwrap();
    let o_send1 = o_send.clone();
    let me_info = info.clone();
    let msg_codec = Arc::new(Mutex::new(PeerWireMessageCodec::new()));
    let me_msg_codec = msg_codec.clone();
    std::thread::spawn(move ||{
        let num= 24*1024;
        let mut in_buffer = Cursor::new(vec![0u8; num]);
        loop {
            let mut read_position = in_buffer.position() as usize;
            //info!("[peer task] in read_position:{:?}",read_position);
            let in_slice = &mut in_buffer.get_mut()[read_position..];
            let read_result = p_recv.read(in_slice);
            if let Ok(bytes_read) = read_result {
                read_position += bytes_read;
            }
            in_buffer.set_position(read_position as u64);

            // Try to parse whatever part of the message we currently have (see if we need to disconnect early)
            let mut data_slice = &in_buffer.get_mut()[..read_position];

            loop {
                let me_msg_code_lock = me_msg_codec.lock();
                if let Ok(mut msg_codec) = me_msg_code_lock {
                    info!("[peer task] read read_position:{:?}",read_position);
                    info!("[peer task] msg_head:{:?}",&data_slice[0..4]);

                    //此处使用 if let 则在接受到 多个数据时只会解析一个,造成卡顿.
                    //此处使用 while let ,在输入缓冲大时可提高性能,但要处理数据不全时 数据头里记录的长度与读取到的长度不相符而导致的断言异常
                    while let Ok(msg) = msg_codec.parse_bytes(Bytes::from(data_slice)){
                        let message_size = msg.message_size();
                        info!("[peer task] message_size:{:?}\n",message_size);

                        data_slice= &data_slice[message_size..];
                        //data_slice= &(in_buffer.get_mut()[msg.message_size()..read_position].to_vec());

                        o_send1.send(OPeerManagerMessage::ReceivedMessage(me_info, msg)).unwrap();

                    }

                    let mut v= data_slice.to_vec();
                    let len = v.len();
                    if len < num {
                        v.append(vec![0_u8;num-len].borrow_mut());
                    }

                    in_buffer = Cursor::new(v);
                    in_buffer.set_position(len as u64);
                    break;
                }
            }
        }
    });

    let p_send = peer;
    let (m_send, m_recv) = mpsc::channel::<IPeerManagerMessage>();
    std::thread::spawn(move || {
        o_send.send(OPeerManagerMessage::PeerAdded(info)).unwrap();
        loop {
            //构造result
            let result = match m_recv.recv() {
                Ok(IPeerManagerMessage::SendMessage(p_info, mid, p_message)) => Ok((
                    Some(p_message),
                    Some(OPeerManagerMessage::SentMessage(p_info, mid)),
                    true,
                )),
                Ok(IPeerManagerMessage::RemovePeer(p_info)) => {
                    Ok((None, Some(OPeerManagerMessage::PeerRemoved(p_info)), false))
                }

                Ok(_) => {
                    info!("bittorrent-protocol_peer: Peer Future Received Invalid Message From Peer Manager");
                    Err(())
                }

                Err(_err) => Ok((None, Some(OPeerManagerMessage::PeerDisconnect(info)), false)),
            };

            //result第一项处理
            let result = match result {
                Ok((opt_send, opt_ack, is_good)) => {
                    if let Some(peer_write_msg) = opt_send {
                        loop {
                            let msg_codec_lock = msg_codec.lock();
                            if let Ok(mut msg_codec)= msg_codec_lock {
                                msg_codec.write_bytes(&peer_write_msg,p_send.try_clone().unwrap()).unwrap();
                                break;
                            }
                        }
                        Ok((opt_ack, is_good))
                    } else {
                        Ok((opt_ack, is_good))
                    }
                }
                Err(_err) => Err(()),
            };

            //result第二项处理
            let result = match result {
                Ok((opt_ack, is_good)) => {
                    if let Some(o_peer_manager_msg) = opt_ack {
                        o_send.send(o_peer_manager_msg).unwrap();
                        Ok(is_good)
                    } else {
                        // Either we had no recv message (from remote), or it was a keep alive message, which we dont propagate
                        Ok(is_good)
                    }
                }
                _ => Err(()),
            };

            //result第三项处理
            match result {
                Ok(is_good) => {
                    // Connection is good if no errors occurred (we do this so we can use the same plumbing)
                    // for sending "acks" back to our manager when an error occurrs, we just have None, None,
                    // Some, false when we want to send an error message to the manager, but terminate the connection.
                    if !is_good {
                        break;
                        //break MergedError::StageThree("草拟马，我要的是处理完后直接退出循环，一直强制我返回一个值，返回你妈呢？")
                    }
                }
                _ => {
                    break;
                }
            }
        } //loop end
    }); // thread end

    m_send
}
