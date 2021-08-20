### Overview

The BitTorrent protocol library and client implemented by the trust language.


### Module description

bittorrent-client-demo： Basic implementation of client.

bittorrent-client：Formal implementation of client (not started).

bittorrent-protocol：Implementation of BT protocol library.

    Basic components:

         Bencode: B code component.

         Metainfo: seed file generation and parsing module.

         Magnet: magnetic link processing module.

         Disk: file fragment storage and loading.


    Peer discovery component:
  
         LSD: local server discovery support.

         Httracker: http tracker server and client code.

         Utracker: UDP tracker server and client code

         DHT: DHT network support.

    File transfer component:

         UTP: implementation of UTP protocol.

         Handshake: handshake information processing module.

         Peer: BT write protocol support.

         Select: extended processing and transmission control


### Version Description

  V0.1: synchronous concurrent implementation

  V0.2: asynchronous concurrent implementation of futures-0.1

  V0.3: asynchronous concurrent implementation of futures-0.3


### Code learning

See examples. Understand the execution process and related class usage, and then look at the source code and testing.


### License

This project is licensed under the [ LGPL 3 license ].

[ LGPL 3 license ]: https://github.com/xin-water/bittorrent-project/blob/master/LICENSE


### Instructions for use

Personal learning use

Please indicate the source again


### Related projects

RustyTorrent：https://github.com/kenpratt/rusty_torrent.

bip-rs: https://github.com/GGist/bip-rs.
