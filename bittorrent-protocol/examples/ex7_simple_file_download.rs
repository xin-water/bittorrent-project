#[macro_use]
extern crate log;

#[macro_use]
extern crate clap;

use std::cell::RefCell;
use std::cmp;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;
use std::sync::mpsc::{self,Sender, Receiver};

use std::sync::{Arc,Mutex};
use chrono::Local;
use log::{LogLevel, LogLevelFilter, LogMetadata, LogRecord};

use bittorrent_protocol::metainfo::{Info, Metainfo};
use bittorrent_protocol::util::bt::PeerId;

use bittorrent_protocol::handshake::{HandshakerManagerBuilder, Extensions, Extension, InitiateMessage, Protocol, HandshakerConfig};
use bittorrent_protocol::handshake::transports::TcpTransport;

use bittorrent_protocol::peer::messages::{
    BitFieldMessage, HaveMessage, PeerWireProtocolMessage, PieceMessage, RequestMessage,
};
use bittorrent_protocol::peer::{
    IPeerManagerMessage, OPeerManagerMessage, PeerInfo, PeerManagerBuilder,
};

use bittorrent_protocol::disk::FileHandleCache;
use bittorrent_protocol::disk::NativeFileSystem;
use bittorrent_protocol::disk::{
    Block, BlockMetadata, BlockMut, DiskManagerBuilder, IDiskMessage, ODiskMessage,
};

/*
    Things this example doesnt do, because of the lack of bittorrent_protocol_select:
      * Logic for piece selection is not abstracted (and is pretty bad)
      * We will unconditionally upload pieces to a peer (regardless whether or not they were choked)
      * We dont add an info hash filter to bittorrent_protocol_handshake after we have as many peers as we need/want
      * We dont do any banning of malicious peers

    Things the example doesnt do, unrelated to bittorrent_protocol_select:
      * Matching peers up to disk requests isnt as good as it could be
      * Doesnt use a shared BytesMut for servicing piece requests
      * Good logging
*/

// How many requests can be in flight at once.
const MAX_PENDING_BLOCKS: usize = 50;

// Some enum to store our selection state updates
#[derive(Debug)]
enum SelectState {
    Choke(PeerInfo),
    UnChoke(PeerInfo),
    Interested(PeerInfo),
    UnInterested(PeerInfo),
    Have(PeerInfo, HaveMessage),
    BitField(PeerInfo, BitFieldMessage),
    NewPeer(PeerInfo),
    RemovedPeer(PeerInfo),
    BlockProcessed,
    GoodPiece(u64),
    BadPiece(u64),
    TorrentSynced,
    TorrentAdded,
}

#[derive(Debug)]
enum Either{
    A(SelectState),
    B(IDiskMessage),
    C(IPeerManagerMessage)
}

fn main() {
    log::set_logger(|m| {
        m.set(LogLevelFilter::max());
        Box::new(SimpleLogger)
    }).unwrap();

    // Command line argument parsing
    let matches = clap_app!(myapp =>
        (version: "1.0")
        (author: "Andrew <amiller4421@gmail.com>")
        (about: "Simple torrent downloading")
        (@arg file: -f +takes_value "Location of the torrent file")
        (@arg dir: -d +takes_value "Download directory to use")
        (@arg peer: -p +takes_value "Single peer to connect to of the form addr:port")
    )
    .get_matches();

    let file = match matches.value_of("file") {
        Some(s) => s,
        None => {
            "bittorrent-protocol/examples_data/torrent/file.torrent"
        }
    };

    let dir = match matches.value_of("dir") {
        Some(s) => s,
        None => "./bittorrent-protocol/examples_data/download",
    };

    let peer_addr = match matches.value_of("peer") {
        Some(s) => s,
        None => "127.0.0.1:44444",
    };

    let peer_addr = peer_addr.parse().unwrap();

    // Load in our torrent file
    let mut metainfo_bytes = Vec::new();
    File::open(file)
        .unwrap()
        .read_to_end(&mut metainfo_bytes)
        .unwrap();

    // Parse out our torrent file
    let metainfo = Metainfo::from_bytes(metainfo_bytes).unwrap();
    let info_hash = metainfo.info().info_hash();
    println!("info-hash:{:?}", hex::encode(&info_hash));

    let peer_id:PeerId = (*b"-UT2060-000000000000").into();

    // Activate the extension protocol via the handshake bits
    let mut extensions = Extensions::new();
    extensions.add(Extension::ExtensionProtocol);

    // Create a handshaker that can initiate connections with peers
    // Create a handshaker that can initiate connections with peers
    let (mut handshaker_send, mut handshaker_recv) = HandshakerManagerBuilder::new()
        .with_peer_id(peer_id)
        // We would ideally add a filter to the handshaker to block
        // peers when we have enough of them for a given hash, but
        // since this is a minimal example, we will rely on peer
        // manager backpressure (handshaker -> peer manager will
        // block when we reach our max peers). Setting these to low
        // values so we dont have more than 2 unused tcp connections.
        .with_config(
            HandshakerConfig::default()
                .with_wait_buffer_size(0)
                .with_done_buffer_size(0),
        )
        .build(TcpTransport) // Will handshake over TCP (could swap this for UTP in the future)
        .unwrap()
        .into_parts();

    // Create a peer manager that will hold our peers and heartbeat/send messages to them
    let (mut peer_manager_send, mut peer_manager_recv) = PeerManagerBuilder::new()
        // Similar to the disk manager sink and stream capacities, we can constrain those
        // for the peer manager as well.
        .with_sink_buffer_capacity(0)
        .with_stream_buffer_capacity(0)
        .build()
        .into_parts();

    // Create a disk manager to handle storing/loading blocks (we add in a file handle cache
    // to avoid anti virus causing slow file opens/closes, will cache up to 100 file handles)
    let (mut disk_manager_send,mut disk_manager_recv) = DiskManagerBuilder::new()
        // Reducing our sink and stream capacities allow us to constrain memory usage
        // (though for spiky downloads, this could effectively throttle us, which is ok too.)
        .with_buffer_capacity(100)
        .build(FileHandleCache::new(
            NativeFileSystem::with_directory(dir),
            100,
        ))
        .into_parts();

    let (select_send, select_recv) = mpsc::channel( );

    // Will hold a mapping of BlockMetadata -> Vec<PeerInfo> to track which peers to send a queued block to
    let disk_request_map:Arc<Mutex<HashMap<BlockMetadata,Vec<PeerInfo>>>> = Arc::new(Mutex::new(HashMap::new()));


    //////////////////////////////////////////////////////////////////////////////////////

    // Hook up a future that feeds incoming (handshaken) peers over to the peer manager
    let mut handshark_peer_manager_send = peer_manager_send.clone();
    std::thread::spawn(move ||{

            let (_, extensions, hash, pid, addr, sock) = handshaker_recv.poll().unwrap().into_parts();

            // Create our peer identifier used by our peer manager
            let peer_info = PeerInfo::new(addr, pid, hash, extensions);

            // Map to a message that can be fed to our peer manager
            handshark_peer_manager_send.send( IPeerManagerMessage::AddPeer(peer_info, sock));

    });

    // Map out the errors for these sinks so they match
    let mut peer_select_send = select_send.clone();
    let mut peer_disk_manager_send = disk_manager_send.clone();
    let mut arc_disk_request_map = disk_request_map.clone();
    // Hook up a future that receives messages from the peer manager, and forwards request to the disk manager or selection manager (using loop fn
    // here because we need to be able to access state, like request_map and a different future combinator wouldnt let us keep it around to access)
    std::thread::spawn(move ||{
        loop {
            let opt_message = match peer_manager_recv.poll().unwrap() {
                OPeerManagerMessage::PeerAdded(info) => {
                    println!("[peer loop]: PeerAdded \n");
                    Some(Either::A(SelectState::NewPeer(info)))
                }
                OPeerManagerMessage::SentMessage(_, _) => None,
                OPeerManagerMessage::PeerRemoved(info) => {
                    println!(
                        "We Removed Peer {:?} \n------------From The Peer Manager",
                        info
                    );
                    Some(Either::A(SelectState::RemovedPeer(info)))
                }
                OPeerManagerMessage::PeerDisconnect(info) => {
                    println!("Peer {:?} \n------------Disconnected From Us", info);
                    Some(Either::A(SelectState::RemovedPeer(info)))
                }
                OPeerManagerMessage::PeerError(info, error) => {
                    println!(
                        "Peer {:?} \n------------Disconnected With Error: {:?}",
                        info, error
                    );
                    Some(Either::A(SelectState::RemovedPeer(info)))
                }
                OPeerManagerMessage::ReceivedMessage(info, message) => {
                        match message {
                            PeerWireProtocolMessage::Choke => {
                                info!("[peer loop ReceivedMessage]: Choke \n");
                                Some(Either::A(SelectState::Choke(info)))
                            }
                            PeerWireProtocolMessage::UnChoke => {
                                info!("[peer loop ReceivedMessage]: UnChoke \n");
                                Some(Either::A(SelectState::UnChoke(info)))
                            }
                            PeerWireProtocolMessage::Interested => {
                                info!("[peer loop ReceivedMessage]: Interested \n");
                                Some(Either::A(SelectState::Interested(info)))
                            }
                            PeerWireProtocolMessage::UnInterested => {
                                info!("[peer loop ReceivedMessage]: UnInterested \n");
                                Some(Either::A(SelectState::UnInterested(info)))
                            }
                            PeerWireProtocolMessage::Have(have) => {
                                info!("[peer loop ReceivedMessage]: Have {:?}\n",&have.piece_index());
                                Some(Either::A(SelectState::Have(info, have)))
                            }
                            PeerWireProtocolMessage::BitField(bitfield) => {
                                info!("[peer loop ReceivedMessage]: bitfield \n");
                                Some(Either::A(SelectState::BitField(info, bitfield)))
                            }
                            PeerWireProtocolMessage::Piece(piece) => {
                                info!("[peer loop ReceivedMessage]: piece_index :oX{:x}, block_offset:oX{:x}, block_length:oX{:x} \n",&piece.piece_index(),&piece.block_offset(),&piece.block_length());
                                let block_metadata = BlockMetadata::new(
                                    info_hash,
                                    piece.piece_index() as u64,
                                    piece.block_offset() as u64,
                                    piece.block_length(),
                                );

                                // Peer sent us a block, send it over to the disk manager to be processed
                                Some(Either::B(IDiskMessage::ProcessBlock(Block::new(
                                    block_metadata,
                                    piece.block(),
                                ))))
                            }
                            PeerWireProtocolMessage::Request(request) => {
                                info!("[peer loop ReceivedMessage]: Request :{:?} \n",&request.piece_index());

                                let block_metadata = BlockMetadata::new(
                                    info_hash,
                                    request.piece_index() as u64,
                                    request.block_offset() as u64,
                                    request.block_length(),
                                );

                                // Lookup the peer info given the block metadata
                                let mut request_map_mut ;
                                loop {
                                    if let Ok(val) = arc_disk_request_map.lock(){
                                        request_map_mut = val;
                                        break
                                    }
                                }

                                // Add the block metadata to our request map, and add the peer as an entry there
                                let block_entry = request_map_mut.entry(block_metadata);
                                let peers_requested = block_entry.or_insert(Vec::new());

                                peers_requested.push(info);

                                Some(Either::B(IDiskMessage::LoadBlock(BlockMut::new(
                                    block_metadata,
                                    vec![0u8; block_metadata.block_length()].into(),
                                ))))
                            }
                            _ => None,
                        }
                }
            };

            // Could optimize out the box, but for the example, this is cleaner and shorter
            match opt_message {
                Some(Either::A(select_message)) => {
                    peer_select_send.send(select_message).unwrap();
                }
                Some(Either::B(disk_message)) => {
                    peer_disk_manager_send.send(disk_message).unwrap();
                }
                _ => {}
            };

        }
    });


    let mut disk_peer_manager_send = peer_manager_send.clone();
    // Map out the errors for these sinks so they match
    let disk_select_send = select_send.clone();
    // Hook up a future that receives from the disk manager, and forwards to the peer manager or select manager
    std::thread::spawn(move | | {
         loop {
             let msg = disk_manager_recv.next().unwrap();
             let opt_message = match msg  {
                 ODiskMessage::TorrentAdded(_) => Some(Either::A(SelectState::TorrentAdded)),
                 ODiskMessage::TorrentSynced(_) => {
                     Some(Either::A(SelectState::TorrentSynced))
                 }
                 ODiskMessage::FoundGoodPiece(_, index) => {
                     Some(Either::A(SelectState::GoodPiece(index)))
                 }
                 ODiskMessage::FoundBadPiece(_, index) => {
                     Some(Either::A(SelectState::BadPiece(index)))
                 }
                 ODiskMessage::BlockProcessed(block) => {
                     info!("[disk] BlockProcessed: piece_index :0X{:x}, block_offset :0X{:x}, block_length :0X{:x},",block.metadata().piece_index(),block.metadata().block_offset(),block.metadata().block_length());
                     Some(Either::A(SelectState::BlockProcessed))
                 }
                 ODiskMessage::BlockLoaded(block) => {
                     let (metadata, block) = block.into_parts();

                     // Lookup the peer info given the block metadata
                     let mut request_map_mut ;
                     loop {
                         if let Ok(val) = disk_request_map.lock(){
                             request_map_mut = val;
                             break
                         }
                     }
                     let mut peer_list = request_map_mut.get_mut(&metadata).unwrap();
                     let peer_info = peer_list.remove(1);

                     // Pack up our block into a peer wire protocol message and send it off to the peer
                     let piece = PieceMessage::new(
                         metadata.piece_index() as u32,
                         metadata.block_offset() as u32,
                         block.freeze(),
                     );
                     let pwp_message = PeerWireProtocolMessage::Piece(piece);

                     Some(Either::C(IPeerManagerMessage::SendMessage(
                         peer_info,
                         0,
                         pwp_message,
                     )))
                 }
                 _ => None,
             };

             // Could optimize out the box, but for the example, this is cleaner and shorter
                 match opt_message {
                     Some(Either::A(select_message)) => {
                         disk_select_send.send(select_message);
                     }

                     Some(Either::C(peer_message)) => {
                         disk_peer_manager_send.send(peer_message);
                     }

                     None => {}
                     _ => {}
                 };

         }
     });

    /////////////////////////////////////////////////////////////////////////////////////////////

    println!(
        "{:?} 添加种子并校验本地数据: send IDiskMessage AddTorrent ",
        Local::now().naive_local()
    );
    // Have our disk manager allocate space for our torrent and start tracking it
    disk_manager_send.send(IDiskMessage::AddTorrent(metainfo.clone()));
    println!(
        "{:?} 添加种子并校验本地数据: complete IDiskMessage AddTorrent",
        Local::now().naive_local()
    );


    println!(
        "{:?} 添加种子并校验本地数据: start 生成块请求队列",
        Local::now().naive_local()
    );
    // Generate data structure to track the requests we need to make, the requests that have been fulfilled, and an active peers list
    let mut piece_requests = generate_requests(metainfo.info(), 16 * 1024);

    println!(
        "{:?} 添加种子并校验本地数据: start 校验本地文件，过滤块请求队列 ",
        Local::now().naive_local()
    );
    let mut cur_pieces = 0;
    //  过滤块请求队列;
    // For any pieces we already have on the file system (and are marked as good), we will be removing them from our requests map
    loop {
       match select_recv.recv().unwrap() {
            // Disk manager identified a good piece already downloaded
            SelectState::GoodPiece(index) => {
                piece_requests = piece_requests
                    .into_iter()
                    .filter(|req| req.piece_index() != index as u32)
                    .collect();

                cur_pieces +=1 ;
            }
            // Disk manager is finished identifying good pieces, torrent has been added
            SelectState::TorrentAdded => {
                break;
            }

            // Shouldnt be receiving any other messages...
            // message => panic!("Unexpected Message Received In Selection Receiver: {:?}", message),
            message => {
                println!(
                    "Unexpected Message Received In Selection Receiver: {:?}",
                    message
                );
            }
        }
    }
    println!(
        "{:?} 添加种子并校验本地数据: end 过滤块请求队列: ",
        Local::now().naive_local()
    );

    let total_pieces = metainfo.info().pieces().count();
    println!(
        "{:?} 添加种子并校验本地数据: Total Pieces: {:?}, Current Pieces: {:?}, Requests Left: {:?}",
        Local::now().naive_local(),
        total_pieces,
        cur_pieces,
        piece_requests.len()
    );

    /////////////////////////////////////////////////////////

    println!("{:?} 下载: 发起握手", Local::now().naive_local());
    // Send the peer given from the command line over to the handshaker to initiate a connection
    handshaker_send.send(InitiateMessage::new(Protocol::BitTorrent, info_hash, peer_addr)).unwrap();

    println!("{:?} 下载: 正在下载 ", Local::now().naive_local());
    // Finally, setup our main event loop to drive the tasks we setup earlier

    let mut opt_peer :Option<PeerInfo>= None;
    let mut unchoked = false;
    let mut blocks_pending = 0;
    loop {
        let msg = select_recv.recv().unwrap();
        info!("[select loop]: {:?}\n", &msg);
        let send_messages = match msg {
            SelectState::BlockProcessed => {
                // Disk manager let us know a block was processed (one of our requests made it
                // from the peer manager, to the disk manager, and this is the acknowledgement)
                blocks_pending -= 1;
                vec![]
            }
            SelectState::Choke(_) => {
                // Peer choked us, cant be sending any requests to them for now
                unchoked = false;
                vec![]
            }
            SelectState::UnChoke(_) => {
                // Peer unchoked us, we can continue sending sending requests to them
                unchoked = true;
                vec![]
            }
            SelectState::NewPeer(info) => {
                // A new peer connected to us, store its contact info (just supported one peer atm),
                // and go ahead and express our interest in them, and unchoke them (we can upload to them)
                // We dont send a bitfield message (just to keep things simple).
                opt_peer = Some(info);
                vec![
                    IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::Interested,
                    ),
                    IPeerManagerMessage::SendMessage(
                        info,
                        0,
                        PeerWireProtocolMessage::UnChoke,
                    ),
                ]
            }
            SelectState::GoodPiece(piece) => {
                // Disk manager has processed endough blocks to make up a piece, and that piece
                // was verified to be good (checksummed). Go ahead and increment the number of
                // pieces we have. We dont handle bad pieces here (since we deleted our request
                // but ideally, we would recreate those requests and resend/blacklist the peer).
                cur_pieces += 1;

                if let Some(peer) = opt_peer {
                    // Send our have message back to the peer
                    vec![IPeerManagerMessage::SendMessage(
                        peer,
                        0,
                        PeerWireProtocolMessage::Have(HaveMessage::new(piece as u32)),
                    )]
                } else {
                    vec![]
                }
            }
            // Decided not to handle these two cases here
            SelectState::RemovedPeer(info) => {
                panic!("Peer {:?} \n----------Got Disconnected", info)
            }
            SelectState::BadPiece(_) => panic!("Peer Gave Us Bad Piece"),
            _ => vec![],
        };


        // Need a type annotation of this return type, provide that
        if cur_pieces == total_pieces {
            // We have all of the (unique) pieces required for our torrent
            break;

        } else if let Some(peer) = opt_peer {
            // We have peer contact info, if we are unchoked, see if we can queue up more requests
            let next_piece_requests = {
                if unchoked {
                    let take_blocks =
                        cmp::min(MAX_PENDING_BLOCKS - blocks_pending, piece_requests.len());
                    blocks_pending += take_blocks;

                    piece_requests
                        .drain(0..take_blocks)
                        .map(move |item| {
                            IPeerManagerMessage::SendMessage(
                                peer,
                                0,
                                PeerWireProtocolMessage::Request(item),
                            )
                        })
                        .collect()
                } else {
                    vec![]
                }

            };

            // First, send any control messages, then, send any more piece requests
            for msg in send_messages.into_iter(){
                //info!("[select loop send msg]: {:?}\n", &msg);
                peer_manager_send.send(msg);
            }

            for msg in next_piece_requests.into_iter(){
                //info!("[select loop send request]: {:?}\n", &msg);
                peer_manager_send.send(msg);
            }

        } else {

        };
    }

    println!("{:?} 下载: 下载完成", Local::now().naive_local());
}

/// Generate a mapping of piece index to list of block requests for that piece, given a block size.
///
/// Note, most clients will drop connections for peers requesting block sizes above 16KB.
fn generate_requests(info: &Info, block_size: usize) -> Vec<RequestMessage> {
    let mut requests = Vec::new();

    // Grab our piece length, and the sum of the lengths of each file in the torrent
    let piece_len: u64 = info.piece_length();
    let mut total_file_length: u64 = info.files().map(|file| file.length()).sum();

    // Loop over each piece (keep subtracting total file length by piece size, use cmp::min to handle last, smaller piece)
    let mut piece_index: u64 = 0;
    while total_file_length != 0 {
        let next_piece_len = cmp::min(total_file_length, piece_len);

        // For all whole blocks, push the block index and block_size
        let whole_blocks = next_piece_len / block_size as u64;
        for block_index in 0..whole_blocks {
            let block_offset = block_index * block_size as u64;

            requests.push(RequestMessage::new(
                piece_index as u32,
                block_offset as u32,
                block_size,
            ));
        }

        // Check for any last smaller block within the current piece
        let partial_block_length = next_piece_len % block_size as u64;
        if partial_block_length != 0 {
            let block_offset = whole_blocks * block_size as u64;

            requests.push(RequestMessage::new(
                piece_index as u32,
                block_offset as u32,
                partial_block_length as usize,
            ));
        }

        // Take this piece out of the total length, increment to the next piece
        total_file_length -= next_piece_len;
        piece_index += 1;
    }

    requests
}

struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, metadata: &LogMetadata) -> bool {
        metadata.level() <= LogLevel::Info
    }

    fn log(&self, record: &LogRecord) {
        if self.enabled(record.metadata()) {
            println!("{:?} - {:?}", record.level(), record.args());
        }
    }
}
