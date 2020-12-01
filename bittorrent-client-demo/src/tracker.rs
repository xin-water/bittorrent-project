use std::str::FromStr;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::net::{UdpSocket, SocketAddr};
use std::{convert, io};
use std::io::Read;
use std::time::Duration;

use rand::Rng;

use reqwest;

use crate::meta_info::MetaInfo;
use crate::tracker_response::{Peer, TrackerResponse};
use crate::tracker_response;

pub fn get_peers(peer_id: &str, torrent: &MetaInfo, listener_port: u16) -> Result<Vec<Peer>, TrackerError> {
    let length_string = torrent.info.size.unwrap().to_string();
    let listener_port_string = listener_port.to_string();
    let mut info_hash_ascii = torrent.info_hash_ascii.clone().unwrap();

    /*       //http tracker 参数
              let link=String::from("http://tracker.opentrackr.org:1337/announce?
              info_hash=%a7%19%ff%3drc%176%88wmo%b6%dcb%b5%de%3fzp
              &peer_id=-RC0001-dw1018438366
              &port=41289
              &uploaded=0
              &downloaded=0
              &left=1670982955
              &event=started
              &key=62ECE99D
              &numwant=200
              &compact=1");
     */

    let htracker_params = vec![
        ("info_hash", info_hash_ascii.as_str()),
        ("peer_id", &peer_id),
        ("port", listener_port_string.as_ref()),
        ("uploaded", "0"),
        ("downloaded", "0"),
        ("left", length_string.as_str()),
        // ("corrupt","0"),
        // ("key",&peer_id),
        ("event", "started"),
        ("numwant", "200"),
        ("compact", "1")
    ];


    /*
      //udp tracker 参数

      偏移 大小 名称 值
      0 64位整数 连接ID
      8 32位整数 动作 1
      12 32位整数 事务ID
      16 20字节字符串 特征码(info_hash)
      36 20字节字符串 客户端ID
      56 64位整数 已下载
      64 64位整数 剩余
      72 64位整数 已上传
      80 32位整数 事件
      84 32位整数 IP地址 0
      88 32位整数 key
      92 32位整数 num_want -1
      96 16位整数 端口
      98
   */

    let info_hash_hex=torrent.info_hash_hex.clone().unwrap();
    let mut utracker_params = HashMap::new();
    utracker_params.insert("info_hash_hex", info_hash_hex.as_str());
    utracker_params.insert("info_hash_ascii", info_hash_ascii.as_str());
    utracker_params.insert("peer_id", &peer_id);
    utracker_params.insert("port", listener_port_string.as_ref());
    utracker_params.insert("uploaded", "0");
    utracker_params.insert("downloaded", "0");
    utracker_params.insert("left", length_string.as_str());
    // params.insert("corrupt","0");
    // utracker_params.insert("key", &peer_id[16..20]);
    utracker_params.insert("event", "started");
    utracker_params.insert("numwant", "200");
    utracker_params.insert("compact", "1");


    let announce_list = match torrent.announce_list.clone() {
        Some(mut v) => v,
        None => vec![vec![torrent.announce.clone().unwrap()]],
    };

    // let announce_list=vec![
    //     vec![String::from("http://95.107.48.115:80/announce")],
    //     vec![String::from("http://54.39.98.124:80/announce")],
    //     vec![String::from("http://78.30.254.12:2710/announce")],
    //     vec![String::from("http://184.105.151.164:6969/announce")],
    //     vec![String::from("https://tracker.gbitt.info:443/announce")],
    //     vec![String::from("udp://109.248.43.36:6969/announce")],
    //
    // ];

    let mut tracker_response_list: Vec<TrackerResponse> = Vec::new();
    for mut announce in announce_list {
        let mut address = announce.pop().unwrap();
        let addr = address.as_str();

        if addr.starts_with("udp") {
            println!("\nudp track----------------------------------------------");
            println!("addr:{}", addr);
            let l1 = addr.split("udp://").collect::<String>();
            let l2 = l1.split("/announce").collect::<String>();
            print!("udp socket:\t\t{}", &l2);
            match udp_tracker(l2.as_str(), &utracker_params, torrent) {
                Ok(tracker_response) => tracker_response_list.push(tracker_response),
                Err(e) => println!("{}", e),
            }
            continue;
        }


        if addr.starts_with("http") {
            println!("\nhttp track----------------------------------------------");

            let link = format!("{}?{}", addr, encode_query_params(&htracker_params));
            println!("link:{}", &link);
            let url = link.as_str();

            let  client =  reqwest::blocking::ClientBuilder::new()
                .connect_timeout(Duration::from_millis(1500))
                .build().unwrap();

            match client.get(&link).send() {
                Err(err) => {
                    println!("链接失败:{:?}", err.to_string());
                }
                Ok(mut response) => {
                    println!("链接成功,status: {}", response.status());
                    if response.status().is_success() {

                        let mut var =vec![0;1024];
                        match  response.read(&mut var){
                            Ok(0) =>{}
                            Ok(n)=>{
                                match TrackerResponse::parse(&var[0..n]) {
                                    Ok(v) => {
                                        tracker_response_list.push(v);
                                    }
                                    _ => {println!("解析错误");}
                                }
                            }
                            _ => { println!("读取错误");}
                        }

                    }else {
                        println!("响应错误");
                    }
                }
            }

            continue;
        }
    }

    let mut peers: Vec<Peer> = vec![];
    for mut tracker_response in tracker_response_list {
        peers.append(&mut tracker_response.peers.unwrap());
    }
    Ok(peers)
}

fn udp_tracker(addr: &str, params_hash: &HashMap<&str, &str, RandomState>, torrent: &MetaInfo) -> Result<TrackerResponse, &'static str> {
    let (client, socket_addr) = get_udpsocket(params_hash.get("port").unwrap()).unwrap();
    let mut ws = Ws::new();
    let param = ws.get_param();
    println!("\nudp connect to {}", addr);
    if let Err(_) = client.connect(addr) {
        return Err("\nudp 连接失败");

    };
    println!("\nudp 发送握手");
    if let Err(_) = client.send(param.as_slice()){
        return Err("\nudp 握手失败");
    }
    let mut buffer = [0; 20];
    println!("等待对方响应");
    client.set_read_timeout(Option::from(Duration::from_secs(3))).unwrap();
    let (num, src) = {
        match client.recv_from(&mut buffer) {
            Ok(v) => v,
            _ => return Err("握手超时!"),
        }
    };
    let mut input = &mut buffer[0..num];
    println!("\n收到握手响应");
    if num < 16 {
        println!("1111111");
        // return tracker_response;
        return Err("握手响应长度错误!");
    }

    if !input[..4].eq(&ws.action) {
        // return tracker_response;
        return Err("握手响应动作错误!");
    }
    if !input[4..8].eq(&ws.conenct_id) {
        // return tracker_response;
        return Err("握手响应id错误!");
    }

    let server_id = &input[8..];
    let connect_id: u32 = rand::thread_rng().gen_range(1_00000, 30_0000);
    let announce = announce_param(connect_id, &server_id, &socket_addr, params_hash, torrent);
    println!("发送peer信息");
    if let Err(_) = client.send(announce.as_slice()){
        return Err("发送peer信息失败");
    }
    let mut buffer2 = [0; 256];
    let (num, src) = {
        match client.recv_from(&mut buffer2) {
            Ok(v) => v,
            _ => return Err("peer响应超时"),
        }
    };
    let mut peer_data = &mut buffer2[0..num];

    println!("\n接受到peer响应信息");
    if num < 20 {
        // return tracker_response;
        return Err("peer响应长度错误!");
    }


    if !peer_data[..4].eq(&hex!("00000001")) {
        // return tracker_response;
        return Err("peer响应动作错误!");
    }

    if !peer_data[4..8].eq(&connect_id.to_le_bytes()) {
        // return tracker_response;
        return Err("peer响应id错误!");
    }
    /*
       announce输出：
        偏移 大小 名称 值
        0 32位整数 动作 1
        4 32位整数 事务ID
        8 32位整数 间隔时间
        12 32位整数 下载人数
        16 32位整数 作种人数
        20 + 6 * n 32位整数 IP整数
        24 + 6 * n 16整数 TCP端口（客户端之间连接）
        20 + 6 * N
    */

    println!("信息检验正确,解析中");
    //TODO
    let time = &peer_data[8..12];

    let download_num = &peer_data[12..16];
    // tracker_response.interval=u32::from_bytes(down_num).unwrap();

    let upload_num = &peer_data[16..20];


    let mut peers = Vec::new();
    let mut n = 20;
    for i in 0..(num - 20) / 6 {
        let p = n + 6;
        let peer = Peer::from_bytes(&peer_data[n..p]);
        peers.push(peer);
        n = p;
    }

    let mut tracker_response = TrackerResponse::default();
    tracker_response.peers = Some(peers);
    println!("解析成功");
    Ok(tracker_response)
}

fn encode_query_params(params: &[(&str, &str)]) -> String {
    let param_strings: Vec<String> = params.iter().map(|&(k, v)| format!("{}={}", k, v)).collect();
    param_strings.join("&")
}


#[derive(Debug)]
struct Ws {
    id: [u8; 8],
    action: [u8; 4],
    conenct_id: [u8; 4],
}

impl Ws {
    pub fn new() -> Self {
        let connect_id: u32 = rand::thread_rng().gen_range(1_00000, 30_0000);
        Ws {
            id: hex!("0000041727101980"),
            action: hex!("00000000"),
            conenct_id: connect_id.to_le_bytes(),
        }
    }
    pub fn get_param(&mut self) -> Vec<u8> {
        let mut param: Vec<u8> = Vec::new();

        param.append(&mut self.id.to_vec());
        param.append(&mut self.action.to_vec());
        param.append(&mut self.conenct_id.to_vec());
        println!("udp track param len:{}", &param.len());
        println!("udp track param request:{:?}", &param);
        param
    }
}

fn announce_param(
    connect_id: u32,
    server_id: &[u8],
    socket_addr: &SocketAddr,
    params_hash: &HashMap<&str, &str, RandomState>,
    torrent: &MetaInfo
) -> Vec<u8> {

    let ss = socket_addr.ip().to_string();
    let mut v: Vec<_> = ss.split('.').collect();
    let mut ip: Vec<u8> = Vec::new();
    for i in 0..(v.len()) {
        let c = v.pop().unwrap();
        let temp = u8::from_str(c).unwrap();
        ip.push(temp);
    }
    ip.reverse();

    let mut announce = Vec::new();
    announce.append(&mut server_id.to_vec());
    announce.append(&mut hex!("00000001").to_vec());
    announce.append(&mut connect_id.to_le_bytes().to_vec());
    announce.append(&mut torrent.info_hash.clone().unwrap());
    announce.append(&mut params_hash.get("peer_id").unwrap().to_string().into_bytes());
    announce.append(&mut 0_u64.to_le_bytes().to_vec());
    announce.append(&mut u64::from_str(params_hash.get("left").unwrap()).unwrap().to_le_bytes().to_vec());
    announce.append(&mut 0_u64.to_le_bytes().to_vec());
    announce.append(&mut 2_u64.to_le_bytes().to_vec());
    announce.append(&mut ip);
    announce.append(&mut rand::random::<u32>().to_le_bytes().to_vec());
    announce.append(&mut 1_u32.to_le_bytes().to_vec());
    announce.append(&mut 1_u32.to_le_bytes().to_vec());
    announce.append(&mut socket_addr.port().to_le_bytes().to_vec());
    announce.append(&mut "/announce".to_string().into_bytes());
    announce
}

fn get_udpsocket(port: &str) -> Option<(UdpSocket, SocketAddr)> {
    // let addr = format!("{}:{}", "0.0.0.0", port);
    let addr = String::from("0.0.0.0:0");
    let client = match UdpSocket::bind(addr.as_str()) {
        Ok(s) => s,
        Err(_) => UdpSocket::bind("127.0.0.1:13844").unwrap(),
    };
    let socket_addr = match client.local_addr() {
        Ok(addr) => addr,
        Err(_) => return None,
    };
    Some((client, socket_addr))
}

#[derive(Debug)]
pub enum TrackerError {
    DecoderError(tracker_response::Error),
   // HyperError(hyper::Error),
    IoError(io::Error),
}

impl convert::From<tracker_response::Error> for TrackerError {
    fn from(err: tracker_response::Error) -> TrackerError {
        TrackerError::DecoderError(err)
    }
}

// impl convert::From<hyper::Error> for TrackerError {
//     fn from(err: hyper::Error) -> TrackerError {
//         TrackerError::HyperError(err)
//     }
// }

impl convert::From<io::Error> for TrackerError {
    fn from(err: io::Error) -> TrackerError {
        TrackerError::IoError(err)
    }
}
