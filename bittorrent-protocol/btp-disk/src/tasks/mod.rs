use std::collections::HashMap;
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
    active_checker: HashMap<InfoHash,Timeout>,
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
            active_checker: HashMap::new(),
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
        let timeout = self.timer.schedule_in(Duration::from_secs(2), [0u8; 20].into());
        self.active_checker.insert([0u8; 20].into(),timeout);
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

   async fn message_in_ex(&mut self, msg: IDiskMessage){
        match msg {
            IDiskMessage::AddTorrent(metainfo) => {
                // 注册一个检查片的定时信号
                // 为啥不在执行内部注册？ 因为我想让任务在单独协程执行，timer加锁太麻烦，还会阻塞线程
                // 以后可以用其他计时器库，基于通道发送 计时信息
                // 当前的一个问题是：种子完成以后怎么取消
                let timeout = self.timer.schedule_in(Duration::from_millis(1800),metainfo.info().info_hash());
                self.active_checker.insert(metainfo.info().info_hash(),timeout);

                execute_add_torrent(metainfo, self.context.clone(), self.out_message.clone()).await
            }
            IDiskMessage::RemoveTorrent(hash) =>  {

                if let Some(timeout) = self.active_checker.remove(&hash){
                    self.timer.cancel(timeout);
                }

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

        let timeout = self.timer.schedule_in(Duration::from_millis(1800),token);
        self.active_checker.insert(token,timeout);

        //使用定时任务来做piece检查，如果收到一个块就检查一次，太浪费cpu了，效率不高。
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


    //  一块一块的存，磁盘访问太频繁，后面应该增加缓存。存缓存里。
    //  存数据时用读锁，存完以后用写锁修改状态
    context.use_torrent_context(info_hash, |metainfo_file, mut checker_state| {
        info!(
            "Processsing Block, Acquired Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );

        let piece_accessor = PieceAccessor::new(context.filesystem(), metainfo_file.info());

        // Write Out Piece Out To The Filesystem And Recalculate The Diff
        if let Ok(_) =piece_accessor.write_piece(&block, &metadata){
            found_hash= true;
        }

        info!(
            "Processsing Block, Released Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );
    });

    if found_hash {

        context.update_torrent_context(info_hash,|_,checker_state|{
            checker_state.add_pending_block(metadata);
        });

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

    //todo 加锁以后校验，锁太长时间了。影响性能。
    context.update_torrent_context(torrent_hash,|metainfo,state_checker|{

        // todo 通过填充虚假片块，然后读一片较验一片，
        // 问题1：太慢了，读的次数太多，不如一次读连续n片，然后一起校验
        // 问题2：一个协程太慢，尝试开多个协程
        // 问题3：校验后没有发送 片状态，没有校验进度。
        state_checker.fill_checker_state(metainfo.info());
        let rs= state_checker.run_with_whole_pieces(metainfo.info(),context.filesystem().clone(),out_message.clone());

        // 新校验算法，还未实现
        // let rs= check_torrent(metainfo.info(),state_checker,out_message.clone(),context.filesystem());

        if let Ok(_) = rs {
           // send_piece_diff(state_checker,torrent_hash,out_message.clone(),false);
            result = true;
        }

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

        if let Ok(_) = state_checker.run_with_whole_pieces(metainfo.info(),context.filesystem(),out_message.clone()){
            //send_piece_diff(state_checker,token,out_message.clone(),false);
        }

    });
}

fn send_piece_diff(
    checker_state: &mut PieceStateChecker,
    hash: InfoHash,
    blocking_sender: mpsc::UnboundedSender<ODiskMessage>,
    include_bad: bool,
) {
    let index_vec= checker_state.run_with_diff(move |piece_state|{
        match (piece_state, include_bad) {
            (PieceState::Good(index), _) =>true ,
            (PieceState::Bad(_), true) => true,
            (PieceState::Bad(index), false) => false,
        }
    });

    for index in index_vec {
        blocking_sender
            .send(ODiskMessage::FoundGoodPiece(hash, index))
            .expect("bittorrent-protocol_disk: Failed To Flush Piece State Message");
    }
}





pub(crate) fn check_torrent<F>(info: &Info, checker: &mut PieceStateChecker, msg_out: mpsc::UnboundedSender<ODiskMessage>,fs: F)->io::Result<()>
where F:FileSystem
{


Ok(())
}
