#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate log;

#[macro_use]
extern crate nom;

extern crate num_traits;

#[cfg(test)]
extern crate quickcheck;

#[macro_use]
extern crate futures;

#[cfg(test)]
extern crate futures_test;

#[macro_use]
extern crate tokio;

pub mod util;

#[macro_use]
pub mod bencode;
pub mod metainfo;
pub mod magnet;
pub mod disk;

pub mod lsd;
pub mod htracker;
pub mod utracker;
pub mod dht;

pub mod utp;
pub mod handshake;
pub mod peer;
pub mod select;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
