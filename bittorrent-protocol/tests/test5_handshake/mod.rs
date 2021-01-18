mod test_byte_after_handshake;
mod test_bytes_after_handshake;
mod test_connect;
mod test_filter_allow_all;
mod test_filter_block_all;
mod test_filter_whitelist_diff_data;
mod test_filter_whitelist_same_data;

//----------------------------------------------------------------------------------//

#[derive(PartialEq, Eq, Debug)]
pub enum TimeoutResult {
    TimedOut,
    GotResult,
}

#[test]
pub fn my_print() {
    println!("test handshake");
}
