use super::message::Message;

#[derive(Clone, Debug)]
pub enum IPC {
    CatComplete,
    BlockComplete(u32, u32),
    PieceComplete(u32),
    DownloadComplete,
    Message(Message),
    BlockUploaded,
    Upstream_Error,
    Downstream_Error,
}
