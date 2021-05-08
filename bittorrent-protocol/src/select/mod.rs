/**
*   拓展消息处理与对等点选择模块
*   设计思路:
*   extend模块是基础,定义了协商拓展消息流程,discovery各子组件是它的插件,负责生成和存储协商拓展的各种参数.extend会依次调用它们. 在process_message 与 poll方法
*   同时extend模块 和 discovery各子组件又是uber的插件, uber接收到消息后会依次调用他们. 在start_sink_state 与 poll_stream_state方法
*/
pub mod revelation;

pub mod discovery;

mod extended;
pub use self::extended::{ExtendedListener, ExtendedPeerInfo, IExtendedMessage, OExtendedMessage};

mod base;
pub use self::base::ControlMessage;

mod uber;
pub use self::uber::{IUberMessage, OUberMessage, UberModule, UberModuleBuilder};

pub mod error;

