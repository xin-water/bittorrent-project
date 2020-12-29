use std::fs;
use std::io::Read;

use bencode;
use bencode::util::ByteString;
use bencode::{Bencode, FromBencode};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use sha1::Sha1;

use crate::decoder;

#[derive(Debug, PartialEq, Clone)]
pub struct Info {
    pub name: String,
    pub length: Option<u64>,
    pub files: Option<Vec<File>>,
    //块的长度(大小)
    pub piece_length: u64,
    //所有块hash值的集合
    pub pieces: Vec<u8>,

    //所有块hash值的集合,由上面属性分段而来,自定义参数
    pub piece_list: Option<Vec<Vec<u8>>>,
    //块的数目,有上面两个属性都可求得,自定义参数
    pub num_pieces: Option<u32>,
    pub md5sum: Option<String>,
    pub private: Option<u8>,
    pub path: Option<Vec<String>>,
    pub root_hash: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct File {
    pub path: Vec<String>,
    pub length: u64,
    pub md5sum: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct MetaInfo {
    pub info: Info,
    pub info_hash: Option<Vec<u8>>,
    pub info_hash_hex: Option<String>,
    pub info_hash_ascii: Option<String>,
    pub announce: Option<String>,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub nodes: Option<Vec<Node>>,
    pub encoding: Option<String>,
    pub httpseeds: Option<Vec<String>>,
    pub creation_date: Option<u64>,
    pub created_by: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Node(String, u64);

impl FromBencode for MetaInfo {
    type Err = decoder::Error;

    fn from_bencode(bencode: &bencode::Bencode) -> Result<MetaInfo, decoder::Error> {
        match bencode {
            &Bencode::Dict(ref m) => {
                let info_bytes = get_field_as_bencoded_bytes!(m, "info");
                let mut sha = Sha1::new();
                sha.update(&info_bytes);
                let info_hash = sha.digest().bytes().to_vec();
                let info_hash_hex = sha.digest().to_string();
                let hex = percent_encode(&info_hash, NON_ALPHANUMERIC).to_string();
                let mut torrent = MetaInfo {
                    announce: get_field!(m, "announce"),
                    announce_list: get_field!(m, "announce-list"),
                    nodes: get_field!(m, "nodes"),
                    encoding: get_field!(m, "encoding"),
                    httpseeds: get_field!(m, "httpseeds"),
                    creation_date: get_field!(m, "creation_date"),
                    created_by: get_field_with_default!(m, "created by", "".to_string()),
                    comment: get_field!(m, "comment"),
                    info: get_field!(m, "info").unwrap(),
                    info_hash: Some(info_hash),
                    info_hash_hex: Some(info_hash_hex),
                    info_hash_ascii: Some(hex),
                };
                Ok(torrent)
            }
            _ => Err(decoder::Error::NotADict),
        }
    }
}

impl FromBencode for Node {
    type Err = decoder::Error;

    fn from_bencode(bencode: &bencode::Bencode) -> Result<Node, decoder::Error> {
        match bencode {
            &Bencode::Dict(ref m) => {
                // let pieces_bytes = get_field_as_bytes!(m, "nodes");
                let node = Node {
                    0: String::from("node"),
                    1: 100,
                };
                Ok(node)
            }
            _ => Err(decoder::Error::NotADict),
        }
    }
}

impl FromBencode for Info {
    type Err = decoder::Error;

    fn from_bencode(bencode: &bencode::Bencode) -> Result<Info, decoder::Error> {
        match bencode {
            &Bencode::Dict(ref m) => {
                let pieces = get_field_as_bytes!(m, "pieces");
                let piece_list: Vec<Vec<u8>> = pieces.chunks(20).map(|v| v.to_owned()).collect();
                let num_pieces = piece_list.len() as u32;

                let length = get_field!(m, "length");
                let files: Option<Vec<File>> = get_field!(m, "files");

                let size = match length {
                    Some(_) => length.clone(),
                    None => {
                        let mut file_size = 0_u64;
                        match &files {
                            Some(f) => {
                                for file in f {
                                    file_size += file.length;
                                }
                                Some(file_size)
                            }
                            None => None,
                        }
                    }
                };
                let info = Info {
                    name: get_field!(m, "name").unwrap(),
                    length,
                    files,
                    size,
                    piece_length: get_field!(m, "piece length").unwrap(),
                    pieces: pieces,
                    piece_list: Some(piece_list),
                    num_pieces: Some(num_pieces),
                    md5sum: get_field!(m, "md5sum"),
                    private: get_field!(m, "private"),
                    path: get_field!(m, "path"),
                    root_hash: get_field!(m, "root_hash"),
                };
                Ok(info)
            }
            _ => Err(decoder::Error::NotADict),
        }
    }
}

impl FromBencode for File {
    type Err = decoder::Error;

    fn from_bencode(bencode: &bencode::Bencode) -> Result<File, decoder::Error> {
        match bencode {
            &Bencode::Dict(ref m) => {
                // let pieces_bytes = get_field_as_bytes!(m, "pieces");
                let file = File {
                    path: get_field!(m, "path").unwrap(),
                    length: get_field!(m, "length").unwrap(),
                    md5sum: get_field!(m, "md5sum"),
                };

                Ok(file)
            }
            _ => Err(decoder::Error::NotADict),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    DecodeError(),
    IoError(),
}

pub fn parse(filename: &str) -> Result<MetaInfo, decoder::Error> {
    // read the torrent file into a byte vector
    let mut f = fs::File::open(filename).expect("load file fail");
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();

    let bencode = bencode::from_vec(v).unwrap();
    let mut result: MetaInfo = FromBencode::from_bencode(&bencode).unwrap();
    render_torrent(&result);
    Ok(result)
}

fn render_torrent(torrent: &MetaInfo) {
    // println!("announce:\t{:?}", torrent.announce);
    // if let &Some(ref al) = &torrent.announce_list {
    //     for a in al {
    //         println!("announce list:\t{}", a[0]);
    //     }
    // }
    // println!("nodes:\t{:?}", torrent.nodes);
    // println!("httpseeds:\t{:?}", torrent.httpseeds);
    // println!("encoding:\t{:?}", torrent.encoding);
    // println!("comment:\t{:?}", torrent.comment);
    // println!("creation date:\t{:?}", torrent.creation_date);
    // println!("created by:\t{:?}", torrent.created_by);
    // println!("name:\t{}", torrent.info.name);
    // println!("length:\t{:?}", torrent.info.length);
    // if let &Some(ref files) = &torrent.info.files {
    //     for f in files {
    //         println!("file path:\t{:?}", f.path);
    //         println!("file length:\t{}", f.length);
    //         println!("file md5sum:\t{:?}", f.md5sum);
    //     }
    // }
    // println!("size:\t{:?}", torrent.info.size);
    // println!("piece length:\t{:?}", torrent.info.piece_length);
    // println!("num_pieces:\t{:?}", torrent.info.num_pieces);

    // println!("pieces:\t{:?}", torrent.info.pieces);
    // println!("piece_list:\t{:?}", torrent.info.piece_list);
    //
    // println!("private:\t{:?}", torrent.info.private);
    // println!("root hash:\t{:?}", torrent.info.root_hash);
    // println!("md5sum:\t{:?}", torrent.info.md5sum);
    // println!("path:\t{:?}", torrent.info.path);

    // println!("info_hash:\t{:?}", torrent.info_hash);
    println!("info_hash_hex:\t{:?}", torrent.info_hash_hex);
    println!("info_hash_ascii:\t{:?}", torrent.info_hash_ascii);
}
