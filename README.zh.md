### 简介

rust 语言实现的 bittorrent 协议库与客户端。


### 模块说明

bittorrent-client-demo：客户端基础实现

bittorrent-client：客户端正式实现（未开始）

bittorrent-protocol：BT协议库实现。

        基础组件：

            bencode：B编码组件。
            
            metainfo：种子文件生成、解析模块。
            
            magnet：磁力链接处理模块。
            
            disk：文件片段存储与加载。

        对等点发现组件：

            lsd：本地服务发现支持。

            htracker：http tracker 服务器与客户端代码。

            utracker：udp tracker 服务器与客户端代码.

            dht: DHT网络支持。

        文件传输组件：

            utp：utp协议实现。 

            nat：地址转换相关。

            handshake：握手信息处理模块。

            peer：BT write 协议支持。

            select：拓展处理 与 传输控制 


### 版本说明

 v0.1: 同步并发实现

 v0.2: futures-0.1 异步并发实现

 v0.3: futures-0.3 异步并发实现


### 代码学习

 查看examples。了解执行过程和相关类用法，然后再看源码与测试。


### 协议

这个项目使用 [ LGPL 3 协议 ].

[ LGPL 3 协议 ]: https://github.com/xin-water/bittorrent-project/blob/master/LICENSE


### 使用须知

 个人学习使用
 
 再次开源请注明出处


### 相关项目

RustyTorrent：https://github.com/kenpratt/rusty_torrent ，bittorrent-client-demo由此修改而来。

bip-rs: https://github.com/GGist/bip-rs ，bittorrent-protocol模块由此修改而来。
