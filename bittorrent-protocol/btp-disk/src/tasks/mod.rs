use crate::error::{
    BlockError, BlockErrorKind, BlockResult, TorrentError, TorrentErrorKind, TorrentResult,
};
use crate::{Block, BlockMut, FileSystem, IDiskMessage, ODiskMessage};
use btp_metainfo::Metainfo;
use btp_util::bt::InfoHash;
use tokio::sync::mpsc;
pub mod context;
use self::context::DiskManagerContext;

mod helpers;
use self::helpers::piece_accessor::PieceAccessor;
use self::helpers::piece_checker::{PieceCheckerMake, PieceStateChecker, PieceState};
use std::sync::Arc;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio::task;

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
    context: DiskManagerContext<F>,
    in_message: mpsc::Receiver<IDiskMessage>,
    out_message: mpsc::UnboundedSender<ODiskMessage>,
}

impl<F: FileSystem> TaskHandler<F>{
    pub(crate) fn new(in_message: mpsc::Receiver<IDiskMessage>,
                      out_message: mpsc::UnboundedSender<ODiskMessage>,fs: F) ->Self{

        TaskHandler{
            context:  DiskManagerContext::new(fs),
            in_message: in_message,
            out_message: out_message,
        }
    }

    pub(crate) async fn run_task(mut self){

        while let Some(msg) = self.in_message.recv().await  {

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
            };
        }
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
    send_piece_diff(&mut init_state_checker, info_hash, blocking_sender.clone(), true);

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
    let found_hash = context.update_torrent_context(info_hash, |metainfo_file, _| {
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

    let mut block_result = Ok(());
    let found_hash = context.update_torrent_context(info_hash, |metainfo_file, mut checker_state| {
        info!(
            "Processsing Block, Acquired Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );

        let piece_accessor = PieceAccessor::new(context.filesystem(), metainfo_file.info());

        // Write Out Piece Out To The Filesystem And Recalculate The Diff
        block_result = piece_accessor.write_piece(&block, &metadata).and_then(|_| {
            checker_state.add_pending_block(metadata);

            PieceCheckerMake::with_state_checker(
                context.filesystem(),
                metainfo_file.info(),
                &mut checker_state,
            )
            .calculate_diff()
        });

        send_piece_diff(&mut checker_state, info_hash, out_message.clone(), false);

        info!(
            "Processsing Block, Released Torrent Lock For {:?}",
            metainfo_file.info().info_hash()
        );
    });

    if found_hash {
        //Ok(block_result?)
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
