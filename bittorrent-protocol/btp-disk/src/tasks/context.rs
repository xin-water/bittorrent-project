use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};

use futures::sink::Sink;
use crate::tasks::helpers::piece_checker::PieceStateChecker;
use crate::ODiskMessage;
use btp_metainfo::Metainfo;
use btp_util::bt::InfoHash;

// 包含文件操作对象
// 种子上下文集合 本质是一个hashmap

// 锁内部的对象是不能为异步的，
// await的时候会一直持有锁，导致其他协程拿不到锁，
// 线程直接卡死，异步没了意义
// 加了双重锁，削弱阻塞范围，针对单个torrent对象的可以使用异步，反正只会锁住它自己
pub struct DiskManagerContext<F> {
    torrent_contexts: Arc<RwLock<HashMap<InfoHash, RwLock<MetainfoState>>>>,
    //out: Sender<ODiskMessage>,
    fs: Arc<F>,
}
#[derive(Clone)]
pub struct MetainfoState {
    file: Metainfo,
    state: PieceStateChecker,
}

impl MetainfoState {
    pub fn new(file: Metainfo, state: PieceStateChecker) -> MetainfoState {
        MetainfoState {
            file: file,
            state: state,
        }
    }
}


impl<F> DiskManagerContext<F> {
    pub fn new(fs: F) -> DiskManagerContext<F> {
        DiskManagerContext {
            torrent_contexts: Arc::new(RwLock::new(HashMap::new())),
            fs: Arc::new(fs),
        }
    }

    // pub fn blocking_sender(&self) -> Sender<ODiskMessage> {
    //     self.out.clone()
    // }

    pub fn filesystem(&self) -> &F {
        &self.fs
    }

    pub fn insert_torrent(&self, file: Metainfo, state: PieceStateChecker) -> bool {
        let mut write_torrents = self.torrent_contexts.write().expect(
            "bittorrent-protocol_disk: DiskManagerContext::insert_torrents Failed To Write Torrent",
        );

        let hash = file.info().info_hash();
        let hash_not_exists = !write_torrents.contains_key(&hash);

        if hash_not_exists {
            write_torrents.insert(hash, RwLock::new(MetainfoState::new(file, state)));
        }

        hash_not_exists
    }

    pub fn update_torrent_context<C>(&self, hash: InfoHash, call: C) -> bool
    where
        C: FnOnce(&Metainfo, &mut PieceStateChecker),
    {
        let read_torrents = self.torrent_contexts.read().expect(
            "bittorrent-protocol_disk: DiskManagerContext::update_torrent Failed To Read Torrent",
        );

        match read_torrents.get(&hash) {
            Some(state) => {
                let mut lock_state = state
                    .write()
                    .expect("bittorrent-protocol_disk: DiskManagerContext::update_torrent Failed To Lock State");
                let deref_state = &mut *lock_state;

                call(&deref_state.file, &mut deref_state.state);

                true
            }
            None => false,
        }
    }

    pub fn use_torrent_context<C>(&self, hash: InfoHash, call: C) -> bool
        where
            C: FnOnce(&Metainfo, &PieceStateChecker),
    {
        let read_torrents = self.torrent_contexts.read().expect(
            "bittorrent-protocol_disk: DiskManagerContext::use_torrent_context Failed To Read Torrent",
        );

        match read_torrents.get(&hash) {
            Some(state) => {
                let mut lock_state = state
                    .read()
                    .expect("bittorrent-protocol_disk: DiskManagerContext::use_torrent_context Failed To Read State");
                let deref_state = & *lock_state;

                call(&deref_state.file, &deref_state.state);

                true
            }
            None => false,
        }
    }

    pub fn remove_torrent(&self, hash: InfoHash) -> bool {
        let mut write_torrents = self.torrent_contexts.write().expect(
            "bittorrent-protocol_disk: DiskManagerContext::remove_torrent Failed To Write Torrent",
        );

        write_torrents.remove(&hash).map(|_| true).unwrap_or(false)
    }
}

impl<F> Clone for DiskManagerContext<F> {
    fn clone(&self) -> DiskManagerContext<F> {
        DiskManagerContext {
            torrent_contexts: self.torrent_contexts.clone(),
            fs: self.fs.clone(),
        }
    }
}
