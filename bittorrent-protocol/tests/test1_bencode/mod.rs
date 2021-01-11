use bittorrent_protocol::bencode::{BDecodeOpt, BencodeRef};

#[test]
pub fn my_print() {
    println!("test bencode");
}

#[test]
fn positive_ben_map_macro() {
    let result = (ben_map! {
        "key" => ben_bytes!("value")
    })
    .encode();
    assert_eq!("d3:key5:valuee".as_bytes(), &result[..]);
    println!("{:?}", String::from_utf8(result));
}

#[test]
fn positive_ben_list_macro() {
    let result = (ben_list!(ben_int!(5))).encode();

    assert_eq!("li5ee".as_bytes(), &result[..]);
    println!("{:?}", String::from_utf8(result));
}

#[test]
fn bench_nested_lists() {
    let bencode = b"lllllllllllllllllllllllllllllllllllllllllllllllllleeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    let result = BencodeRef::decode(&bencode[..], BDecodeOpt::new(50, true, true)).unwrap();
    println!("{:?}", result);
}

#[test]
fn bench_multi_kb_bencode() {
    let bencode = include_bytes!("multi_kb.bencode");

    let result = BencodeRef::decode(&bencode[..], BDecodeOpt::default()).unwrap();
    println!("{:?}", result);
}
