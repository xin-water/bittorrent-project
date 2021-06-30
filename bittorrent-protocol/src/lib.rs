#[macro_use]
extern crate error_chain;

#[macro_use]
extern crate log;

#[macro_use]
extern crate nom;

extern crate num_traits;

#[cfg(test)]
extern crate quickcheck;

pub mod util;

#[macro_use]
pub mod bencode;
pub mod disk;
pub mod magnet;
pub mod metainfo;

pub mod dht;
pub mod htracker;
pub mod lsd;
pub mod utracker;

pub mod handshake;
pub mod peer;
pub mod select;
pub mod utp;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
