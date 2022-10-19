use std::io;
use crate::error::{
    BlockError, BlockErrorKind, BlockResult, TorrentError, TorrentErrorKind, TorrentResult,
};
use crate::{Block, BlockMetadata, BlockMut, FileSystem, IDiskMessage, ODiskMessage};
use btp_metainfo::{Info, Metainfo};
use btp_util::bt::InfoHash;
use tokio::sync::mpsc;
pub mod context;
use self::context::DiskManagerContext;

mod helpers;
use self::helpers::piece_accessor::PieceAccessor;
use self::helpers::piece_checker::{PieceCheckerMake, PieceStateChecker, PieceState};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio::{select, task};
use btp_util::timer::{Timer,Timeout};
use futures::{stream,StreamExt};
use crate::tasks::helpers::piece_checker::last_piece_size;

pub(crate)  fn start_disk_task<F>(sink_capacity: usize,stream_capacity:usize, fs: F) ->(mpsc::Sender<IDiskMessage>,mpsc::UnboundedReceiver<ODiskMessage>)
    where F: std::marker::Sync + std::marker::Send + 'static,
          F: FileSystem
{

    let (in_message_tx,in_message_rx)= mpsc::channel(sink_capacity);
    let (out_message_tx,out_message_rx)= mpsc::unbounded_channel();

    let mut task_handler =TaskHandler::new(in_message_rx, out_message_tx, fs);

    task::spawn(task_handler.run_task());

    return (in_message_tx,out_message_rx);

}

pub(crate) struct TaskHandler<F>{
    is_run: bool,
    context: DiskManagerContext<F>,
    //timer: Arc<Mutex<Timer<InfoHash>>>,
    timer: Timer<InfoHash>,
    in_message: mpsc::Receiver<IDiskMessage>,
    out_message: mpsc::UnboundedSender<ODiskMessage>,
}

impl<F: FileSystem> TaskHandler<F>{
    pub(crate) fn new(in_message: mpsc::Receiver<IDiskMessage>,
                      out_message: mpsc::UnboundedSender<ODiskMessage>,fs: F) ->Self{

        TaskHandler{
            is_run :true,
            context:  DiskManagerContext::new(fs),
            in_message: in_message,
           // timer: Arc::new(Mutex::new(Timer::new())),
            timer:Timer::new(),
            out_message: out_message,
        }
    }

    fn shutdown(&mut self){
        self.is_run = false;
    }

    pub(crate) async fn run_task(mut self){
        // {
        //     if let Ok(mut t) = self.timer.lock(){
        //         (*t).schedule_in(Duration::from_secs(2), [0u8; 20].into());
        //     }
        // }
        self.timer.schedule_in(Duration::from_secs(2), [0u8; 20].into());
        while  self.is_run {
          self.run_one().await;
        }

    }

   async fn run_one(&mut self){
        select! {
           msg  = self.in_message.recv() => {
                if let Some(message) = msg {
                    self.message_in_ex(message).await
                } else {
                    self.shutdown()
                }
           }

           // 这里要开新协程，则timer需要Arc,Mutex,这里要先拿锁再next.
           // token = async {
           //      let mut timer = self.timer.lock().unwrap();
           //      (*timer).next().await
           //  }
           token = self.timer.next(), if !self.timer.is_empty() => {
                let token = token.unwrap();
                self.timeout_ex(token).await
           }


        }
    }

   async fn message_in_ex(&self,msg: IDiskMessage){
        match msg {
            IDiskMessage::AddTorrent(metainfo) => {
                execute_add_torrent(metainfo, self.context.clone(), self.out_message.clone()).await
            }
            IDiskMessage::RemoveTorrent(hash) =>  {
                execute_remove_torrent(hash, self.context.clone(),self.out_message.clone()).await
            },
            IDiskMessage::SyncTorrent(hash) =>  {
                execute_sync_torrent(hash, self.context.clone(),self.out_message.clone()).await
            },
            IDiskMessage::LoadBlock(block) =>   {
                execute_load_block(block, self.context.clone(),self.out_message.clone()).await
            },
            IDiskMessage::ProcessBlock(block) => {
                execute_process_block(block, self.context.clone(), self.out_message.clone()).await
            }
            IDiskMessage::CheckTorrent(info) => {
                execute_check_torrent(info, self.context.clone(), self.out_message.clone()).await
            }
        };
    }

    async  fn timeout_ex(&mut self,token: InfoHash){
        execute_piece_check(token, self.context.clone(), self.out_message.clone()).await
    }
}




async fn execute_add_torrent<F>(
    file: Metainfo,
    context: DiskManagerContext<F>,
    blocking_sender: mpsc::UnboundedSender<ODiskMessage>,
)
where
    F: FileSystem,
{
    let info_hash = file.info().info_hash();
    let mut init_state_checker = PieceCheckerMake::init_state_checker(context.filesystem(), file.info()).expect("PieceChecker init_state error");

    info!("PieceChecker init_state complete ");

    // In case we are resuming a download, we need to send the diff for the newly added torrent
    // send_piece_diff(&mut init_state_checker, info_hash, blocking_sender.clone(), true);

    if context.insert_torrent(file, init_state_checker) {
        blocking_sender
            .send(ODiskMessage::TorrentAdded(info_hash))
            .expect("execute_add_torrent send message fail");
    } else {
        blocking_sender
            .send(ODiskMessage::TorrentError(info_hash, TorrentError::from_kind(
            TorrentErrorKind::ExistingInfoHash { hash: info_hash },
        )))
            .expect("execute_add_torrent send message fail");
    }
}

async fn execute_remove_torrent<F>(hash: InfoHash, context: DiskManagerContext<F>,out_message: UnboundedSender<ODiskMessage>)
where
    F: FileSystem,
{
    if context.remove_torrent(hash) {
        out_message
            .send(ODiskMessage::TorrentRemoved(hash))
            .expect("execute_remove_torrent send message fail");

    } else {

        out_message
            .send(ODiskMessage::TorrentError(hash, TorrentError::from_kind(
                TorrentErrorKind::InfoHashNotFound { hash: hash },
            )))
            .expect("execute_remove_torrent send message fail");
    }
}

async fn execute_sync_torrent<F>(hash: InfoHash, context: DiskManagerContext<F>,out_message:UnboundedSender<ODiskMessage>)
where
    F: FileSystem,
{
    let filesystem = context.filesystem();

    let mut sync_result = Ok(());
    let found_hash = context.update_torrent_context(hash, |metainfo_file, _| {
        let opt_parent_dir = metainfo_file.info().directory();

        for file in metainfo_file.info().files() {
            let path = helpers::build_path(opt_parent_dir, file);

            sync_result = filesystem.sync_file(path);
        }
    });

    if found_hash {
        //Ok(sync_result?)
        out_message
            .send(ODiskMessage::TorrentSynced(hash))
            .expect("execute_sync_torrent send message fail");
    } else {

        out_message
            .send(ODiskMessage::TorrentError(hash, TorrentError::from_kind(
            TorrentErrorKind::InfoHashNotFound { hash: hash },
        )))
            .expect("execute_sync_torrent send message fail");
    }
}

async fn execute_load_block<F>(mut block: BlockMut, context: DiskManagerContext<F>,out_message:mpsc::UnboundedSender<ODiskMessage>)
where
    F: FileSystem,
{
    let metadata = block.metadata();
    let info_hash = metadata.info_hash();

    let mut access_result = Ok(());
    let found_hash = context.use_torrent_context(info_hash, |metainfo_file, _| {
        let piece_accessor = PieceAccessor::new(context.filesystem(), metainfo_file.info());

        // Read The Piece In From The Filesystem
        access_result = piece_accessor.read_piece(&mut *block, &metadata)
    });

    if found_hash {
        //Ok(access_result?)
        out_message
            .send(ODiskMessage::BlockLoaded(block))
            .expect("execute_load_block send message fail");

    } else {

        out_message
            .send(ODiskMessage::LoadBlockError(block, BlockError::from_kind(BlockErrorKind::InfoHashNotFound {
                hash: info_hash,
            })))
            .expect("execute_load_block send message fail");
    }
}

async fn execute_process_block<F>(
    mut block: Block,
    context: DiskManagerContext<F>,
    out_message: mpsc::UnboundedSender<ODiskMessage>,
)
where
    F: FileSystem,
{
    let metadata = block.metadata();
    let info_hash = metadata.info_hash();

    let mut found_hash = false;

    context.update_torrent_context(info_hash, |metainfo_file, mut checker_state| {
        info!(
            "Processsing Block, Acquired Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );

        let piece_accessor = PieceAccessor::new(context.filesystem(), metainfo_file.info());

        // Write Out Piece Out To The Filesystem And Recalculate The Diff
        if let Ok(_) =piece_accessor.write_piece(&block, &metadata){
            checker_state.add_pending_block(metadata);
            // calculate_diff(metainfo_file.info(),&mut checker_state,context.filesystem())
            //send_piece_diff(&mut checker_state, info_hash, out_message.clone(), false);
            found_hash= true;
        }

        info!(
            "Processsing Block, Released Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );
    });

    if found_hash {
        out_message
            .send(ODiskMessage::BlockProcessed(block))
            .expect("execute_process_block send message fail");
    } else {
        out_message
            .send(ODiskMessage::ProcessBlockError(block, BlockError::from_kind(BlockErrorKind::InfoHashNotFound {
                hash: info_hash,
            })))
            .expect("execute_process_block send message fail");
    }
}

async fn execute_check_torrent<F>(torrent_hash: InfoHash,
    context: DiskManagerContext<F>,
    out_message: mpsc::UnboundedSender<ODiskMessage>,
)
where
    F: FileSystem,
{

    let mut result = false;
    context.update_torrent_context(torrent_hash,|metainfo,state_checker|{
          fill_checker_state(metainfo.info(),state_checker);
          calculate_diff(metainfo.info(),state_checker,context.filesystem());
          send_piece_diff(state_checker,torrent_hash,out_message.clone(),false);
          result = true;
    });

    if result {
        out_message.send(
            ODiskMessage::CheckTorrented(torrent_hash)
        ).expect("execute_check_torrent  CheckInfoHashed send mess fail");
    }else {
        out_message.send(
            ODiskMessage::CheckTorrentError(torrent_hash)
        ).expect("execute_check_torrent  CheckInfoHashed send mess fail");
    }
}




async fn execute_piece_check<F>(
    token: InfoHash,
    context: DiskManagerContext<F>,
    out_message: mpsc::UnboundedSender<ODiskMessage>,
)
    where
        F: FileSystem,
{
    context.update_torrent_context(token,|metainfo,state_checker|{
       // fill_checker_state(metainfo.info(),state_checker);
        calculate_diff(metainfo.info(),state_checker,context.filesystem());
        send_piece_diff(state_checker,token,out_message.clone(),false);
    });
}

fn send_piece_diff(
    checker_state: &mut PieceStateChecker,
    hash: InfoHash,
    blocking_sender: mpsc::UnboundedSender<ODiskMessage>,
    ignore_bad: bool,
) {
    let index_vec= checker_state.run_with_diff(move |piece_state|{
        match (piece_state, ignore_bad) {
            (PieceState::Good(index), _) =>true ,
            (PieceState::Bad(index), false) => true,
            (PieceState::Bad(_), true) => false,
        }
    });

    for index in index_vec {
        blocking_sender
            .send(ODiskMessage::FoundGoodPiece(hash, index))
            .expect("bittorrent-protocol_disk: Failed To Flush Piece State Message");
    }
}


// 初始化片状态，存放到等待列表中
/// Fill the PieceCheckerState with all piece messages for each file in our info dictionary.
///
/// This is done once when a torrent file is added to see if we have any good pieces that
/// the caller can use to skip (if the torrent was partially downloaded before).
fn fill_checker_state(info_dict: &Info, state_checker: &mut PieceStateChecker) -> io::Result<()> {
    let piece_length = info_dict.piece_length() as u64;
    let total_bytes: u64 = info_dict
        .files()
        .map(|file| file.length() as u64)
        .sum();

    let full_pieces = total_bytes / piece_length;
    let last_piece_size = last_piece_size(info_dict);

    for piece_index in 0..full_pieces {
        state_checker
            .add_pending_block(BlockMetadata::with_default_hash(
                piece_index,
                0,
                piece_length as usize,
            ));
    }

    // 最后片长度不等于标准片长，说明余一片要加进去
    if last_piece_size != (piece_length as usize)  {
        state_checker
            .add_pending_block(BlockMetadata::with_default_hash(
                full_pieces,
                0,
                last_piece_size as usize,
            ));
    }

    Ok(())
}

// 校验片队列中每一个片的完成情况。
/// Calculate the diff of old to new good/bad pieces and store them in the piece checker state
/// to be retrieved by the caller.
pub fn calculate_diff<T>(info_dict: &Info, state_checker: &mut PieceStateChecker,fs: T) -> io::Result<()>
where  T: FileSystem
{
    let piece_length = info_dict.piece_length() as u64;
    // TODO: Use Block Allocator
    let mut piece_buffer = vec![0u8; piece_length as usize];

    let piece_accessor = PieceAccessor::new(fs, info_dict);

    //片的长度应该放在校验器里。
    state_checker.run_with_whole_pieces(piece_length as usize, |message| {
        log::trace!("check piece: {:?}",message.piece_index());
        // 读取传递过来的块，判断它与片hash是否一样，除非传递过来的是块片，
        // 这里可以直接改为读取 片的长度。
        piece_accessor.read_piece(&mut piece_buffer[..message.block_length()], message)?;

        let calculated_hash = InfoHash::from_bytes(&piece_buffer[..message.block_length()]);
        let expected_hash = InfoHash::from_hash(
            info_dict
                .pieces()
                .skip(message.piece_index() as usize)
                .next()
                .expect("bittorrent-protocol_peer: Piece Checker Failed To Retrieve Expected Hash"),
        )
            .expect("bittorrent-protocol_peer: Wrong Length Of Expected Hash Received");

        Ok(calculated_hash == expected_hash)
    })?;

    Ok(())
}
