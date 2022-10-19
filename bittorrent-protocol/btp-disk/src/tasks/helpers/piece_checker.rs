use std::cmp;
use std::collections::{HashMap, HashSet};
use std::io;

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use crate::error::{TorrentError, TorrentErrorKind, TorrentResult};
use crate::tasks::helpers;
use crate::tasks::helpers::piece_accessor::PieceAccessor;
use crate::{BlockMetadata, FileSystem, ODiskMessage};
use btp_metainfo::Info;
use btp_util::bt::InfoHash;

/// Calculates hashes on existing files within the file system given and reports good/bad pieces.
pub struct PieceCheckerMake<'a, F> {
    fs: F,
    info_dict: &'a Info,
    state_checker: &'a mut PieceStateChecker,
}

impl<'a, F> PieceCheckerMake<'a, F>
where
    F: FileSystem + 'a,
{
    /// Create the initial PieceCheckerState for the PieceChecker.
    pub fn init_state_checker(fs: F, info_dict: &'a Info) -> TorrentResult<PieceStateChecker> {
        let total_blocks = info_dict.pieces().count();
        let last_piece_size = last_piece_size(info_dict);

        let mut state_checker = PieceStateChecker::new(total_blocks, last_piece_size);
        {
            let mut piece_checker_make = PieceCheckerMake::with_state_checker(fs, info_dict, &mut state_checker);

            //校验所有文件，不存在则创建，并在每个文件最后一个字节写入数据0,
            piece_checker_make.validate_files_sizes()?;

            //根据info字典初始化文件块，放到等待队列中，块与片一样大
            //piece_checker_make.fill_checker_state()?;

            //校验等待队列中的块，检查片是否完整。
            //piece_checker_make.calculate_diff()?;
        }

        Ok(state_checker)
    }

    /// Create a new PieceChecker with the given state.
    pub fn with_state_checker(
        fs: F,
        info_dict: &'a Info,
        checker_state: &'a mut PieceStateChecker,
    ) -> PieceCheckerMake<'a, F> {
        PieceCheckerMake {
            fs: fs,
            info_dict: info_dict,
            state_checker: checker_state,
        }
    }

    /// Validates the file sizes for the given torrent file and block allocates them if they do not exist.
    ///
    /// This function will, if the file does not exist, or exists and is zero size, fill the file with zeroes.
    /// Otherwise, if the file exists and it is of the correct size, it will be left alone. If it is of the wrong
    /// size, an error will be thrown as we do not want to overwrite and existing file that maybe just had the same
    /// name as a file in our dictionary.
    fn validate_files_sizes(&mut self) -> TorrentResult<()> {
        for file in self.info_dict.files() {
            let file_path = helpers::build_path(self.info_dict.directory(), file);
            let expected_size = file.length() as u64;

            self.fs
                .open_file(file_path.clone())
                .map_err(|err| err.into())
                .and_then(|mut file| {
                    // File May Or May Not Have Existed Before, If The File Is Zero
                    // Length, Assume It Wasn't There (User Doesn't Lose Any Data)
                    let actual_size = self.fs.file_size(&file)?;

                    let size_matches = actual_size == expected_size;
                    let size_is_zero = actual_size == 0;

                    //文件不存在时
                    if !size_matches && size_is_zero {
                        self.fs
                            .write_file(&mut file, expected_size - 1, &[0])
                            .expect(
                            "bittorrent-protocol_peer: Failed To Create File When Validating Sizes",
                        );
                    // 文件存在但不完整时
                    } else if !size_matches {
                        return Err(TorrentError::from_kind(
                            TorrentErrorKind::ExistingFileSizeCheck {
                                file_path: file_path,
                                expected_size: expected_size,
                                actual_size: actual_size,
                            },
                        ));
                    }

                    Ok(())
                })?;
        }

        Ok(())
    }
}

pub(crate)  fn last_piece_size(info_dict: &Info) -> usize {
    let piece_length = info_dict.piece_length() as u64;
    let total_bytes: u64 = info_dict.files().map(|file| file.length() as u64).sum();

    (total_bytes % piece_length) as usize
}

// ----------------------------------------------------------------------------//

/// Stores state for the PieceChecker between invocations.
#[derive(Clone)]
pub struct PieceStateChecker {
    new_states: Vec<PieceState>,
    old_states: HashSet<PieceState>,
    pending_blocks: HashMap<u64, Vec<BlockMetadata>>,
    total_blocks: usize,
    last_block_size: usize,
}

#[derive(PartialEq, Eq, Hash,Clone)]
pub enum PieceState {
    /// Piece was discovered as good.
    Good(u64),
    /// Piece was discovered as bad.
    Bad(u64),
}

impl PieceState{
    pub(crate) fn get_index(&self)->u64{
        match self{
         &PieceState::Good(index)=>index,
         &PieceState::Bad(index)=>index,
        }
    }
}

impl PieceStateChecker {
    /// Create a new PieceCheckerState.
    pub fn new(total_blocks: usize, last_block_size: usize) -> PieceStateChecker {
        PieceStateChecker {
            new_states: Vec::new(),
            old_states: HashSet::new(),
            pending_blocks: HashMap::new(),
            total_blocks: total_blocks,
            last_block_size: last_block_size,
        }
    }

    /// Add a pending piece block to the current pending blocks.
    pub fn add_pending_block(&mut self, msg: BlockMetadata) {
        self.pending_blocks
            .entry(msg.piece_index())
            .or_insert(Vec::new())
            .push(msg);
    }

    /// Run the given closures against NewGood and NewBad messages. Each of the messages will
    /// then either be dropped (NewBad) or converted to OldGood (NewGood).
    pub fn run_with_diff<F>(&mut self, mut callback: F)->Vec<u64>
    where
        F: Fn(PieceState)-> bool,
    {
         let mut result =Vec::new();
        for piece_state in self.new_states.drain(..) {

            if callback(piece_state.clone()){
                result.push(piece_state.get_index());
            }

            self.old_states.insert(piece_state);
        }
        info!("run_with_diff complete");
        result
    }

    // 校验片队列中每一个片的完成情况。
    /// Pass any pieces that have not been identified as OldGood into the callback which determines
    /// if the piece is good or bad so it can be marked as NewGood or NewBad.
    pub(crate) fn run_with_whole_pieces<F>(&mut self, info_dict: &Info, fs: F, msg_out: mpsc::UnboundedSender<ODiskMessage>) -> io::Result<()>
    where
        F: FileSystem,
    {
        self.merge_pieces();

        let new_states = &mut self.new_states;
        let old_states = &self.old_states;

        let total_blocks = self.total_blocks;
        let last_block_size = self.last_block_size;

        let piece_accessor = PieceAccessor::new(fs, info_dict);
        let piece_length =  info_dict.piece_length() as usize;
        let mut piece_buffer = vec![0u8; piece_length];

        // 从等待列表里 过虑出现在已完成的、旧状态里未完成的块： 新完成的块
        for messages in self
            .pending_blocks
            .values_mut()
            // 过虑出完整片，此时的块与片一样长
            .filter(|ref messages| {
                piece_is_complete(total_blocks, last_block_size, piece_length, messages)
            })
            // 旧状态里面非完整的片。
            .filter(|ref messages| {
                !old_states.contains(&PieceState::Good(messages[0].piece_index()))
            })
        {

            let message = &messages[0];
            log::trace!("check piece: {:?}",message.piece_index());

            // 读取片，因为该块已经聚合，跟片一样大了。
            piece_accessor.read_piece(&mut piece_buffer[..], message)?;

            let calculated_hash = InfoHash::from_bytes(&piece_buffer[..]);
            let expected_hash = InfoHash::from_hash(
                info_dict
                    .pieces()
                    .skip(message.piece_index() as usize)
                    .next()
                    .expect("bittorrent-protocol_peer: Piece Checker Failed To Retrieve Expected Hash"),
            )
                .expect("bittorrent-protocol_peer: Wrong Length Of Expected Hash Received");

            if calculated_hash == expected_hash {
                new_states.push(PieceState::Good(messages[0].piece_index()));
                msg_out.send(ODiskMessage::FoundGoodPiece(
                    info_dict.info_hash(),
                    messages[0].piece_index())
                ).expect("run_with_whole_pieces FoundGoodPiece message fail ")
            } else {
                new_states.push(PieceState::Bad(messages[0].piece_index()));
                msg_out.send(ODiskMessage::FoundBadPiece(
                    info_dict.info_hash(),
                    messages[0].piece_index())
                ).expect("run_with_whole_pieces FoundGoodPiece message fail ")
            }

            // TODO: Should do a partial clear if user callback errors.
            messages.clear();
        }

        Ok(())
    }

    /// Merges all pending piece messages into a single messages if possible.
    fn merge_pieces(&mut self) {
        for (_, ref mut messages) in self.pending_blocks.iter_mut() {
            // Sort the messages by their block offset
            messages.sort_by(|a, b| a.block_offset().cmp(&b.block_offset()));

            let mut messages_len = messages.len();
            let mut merge_success = true;
            // See if we can merge all messages into a single message
            while merge_success && messages_len > 1 {
                let actual_last = messages
                    .pop()
                    .expect("bittorrent-protocol_peer: Failed To Merge Blocks");
                let second_last = messages
                    .pop()
                    .expect("bittorrent-protocol_peer: Failed To Merge Blocks");

                let opt_merged = merge_piece_messages(&second_last, &actual_last);
                if let Some(merged) = opt_merged {
                    messages.push(merged);
                } else {
                    messages.push(second_last);
                    messages.push(actual_last);

                    merge_success = false;
                }

                messages_len = messages.len();
            }
        }
    }
}

/// Merge a piece message a with a piece message b if possible.
///
/// First message's block offset should come before (or at) the block offset of the second message.
fn merge_piece_messages(
    message_a: &BlockMetadata,
    message_b: &BlockMetadata,
) -> Option<BlockMetadata> {
    if message_a.info_hash() != message_b.info_hash()
        || message_a.piece_index() != message_b.piece_index()
    {
        return None;
    }
    let info_hash = message_a.info_hash();
    let piece_index = message_a.piece_index();

    // Check if the pieces overlap
    let start_a = message_a.block_offset();
    let end_a = start_a + message_a.block_length() as u64;

    let start_b = message_b.block_offset();
    let end_b = start_b + message_b.block_length() as u64;

    // If start b falls between start and end a, then start a is where we start, and we end at the max of end a
    // or end b, then calculate the length from end minus start. Vice versa if a falls between start and end b.
    if start_b >= start_a && start_b <= end_a {
        let end_to_take = cmp::max(end_a, end_b);
        let length = end_to_take - start_a;

        Some(BlockMetadata::new(
            info_hash,
            piece_index,
            start_a,
            length as usize,
        ))
    } else if start_a >= start_b && start_a <= end_b {
        let end_to_take = cmp::max(end_a, end_b);
        let length = end_to_take - start_b;

        Some(BlockMetadata::new(
            info_hash,
            piece_index,
            start_b,
            length as usize,
        ))
    } else {
        None
    }
}


// 判断片是否完整，根据块长度来判断。
/// True if the piece is ready to be hashed and checked (full) as good or not.
fn piece_is_complete(
    total_blocks: usize,
    last_block_size: usize,
    piece_length: usize,
    messages: &[BlockMetadata],
) -> bool {
    let is_single_message = messages.len() == 1;
    let is_piece_length = messages
        .get(0)
        .map(|message| message.block_length() == piece_length)
        .unwrap_or(false);
    let is_last_block = messages
        .get(0)
        .map(|message| message.piece_index() == (total_blocks - 1) as u64)
        .unwrap_or(false);
    let is_last_block_length = messages
        .get(0)
        .map(|message| message.block_length() == last_block_size)
        .unwrap_or(false);

    is_single_message && (is_piece_length || (is_last_block && is_last_block_length))
}


#[cfg(test)]
mod tests {

    use crate::BlockMetadata;
    use btp_util::bt;

    #[test]
    fn positive_merge_duplicate_messages() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_a);

        assert_eq!(metadata_a, merged.unwrap());
    }

    #[test]
    fn negative_merge_duplicate_messages_diff_hash() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);
        let metadata_b = BlockMetadata::new([1u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_b);

        assert_eq!(None, merged);
    }

    #[test]
    fn negative_merge_duplicate_messages_diff_index() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);
        let metadata_b = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 1, 5, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_b);

        assert_eq!(None, merged);
    }

    #[test]
    fn positive_merge_no_overlap_messages() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);
        let metadata_b = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 11, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_b);

        assert_eq!(None, merged);
    }

    #[test]
    fn positive_merge_overlap_messages() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);
        let metadata_b = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 8, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_b);
        let expected = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 8);

        assert_eq!(expected, merged.unwrap());
    }

    #[test]
    fn positive_merge_neighbor_messages() {
        let metadata_a = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 5);
        let metadata_b = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 10, 5);

        let merged = super::merge_piece_messages(&metadata_a, &metadata_b);
        let expected = BlockMetadata::new([0u8; bt::INFO_HASH_LEN].into(), 0, 5, 10);

        assert_eq!(expected, merged.unwrap());
    }
}
