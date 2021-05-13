use crate::peer::messages::builders::ExtendedMessageBuilder;
use crate::select::discovery::error::DiscoveryError;
use crate::select::discovery::{IDiscoveryMessage, ODiscoveryMessage};
use futures::Poll;
use futures::Sink;
use futures::StartSend;
use futures::Stream;
use futures::{Async, AsyncSink};

use crate::select::error::UberError;
use crate::select::extended::ExtendedModule;
use crate::select::{ControlMessage, ExtendedListener, IExtendedMessage, OExtendedMessage};

pub trait DiscoveryTrait:
    ExtendedListener
    + Sink<SinkItem = IDiscoveryMessage, SinkError = DiscoveryError>
    + Stream<Item = ODiscoveryMessage, Error = DiscoveryError>
    + Send
{
}

impl<T> DiscoveryTrait for T where
    T: ExtendedListener
        + Sink<SinkItem = IDiscoveryMessage, SinkError = DiscoveryError>
        + Stream<Item = ODiscoveryMessage, Error = DiscoveryError>
        + Send
{
}

/// Enumeration of uber messages that can be sent to the uber module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IUberMessage {
    /// Broadcast a control message out to all modules.
    Control(ControlMessage),
    /// Send an extended message to the extended module.
    Extended(IExtendedMessage),
    /// Send a discovery message to all discovery modules.
    Discovery(IDiscoveryMessage),
}

/// Enumeration of uber messages that can be received from the uber module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OUberMessage {
    /// Receive an extended message from the extended module.
    Extended(OExtendedMessage),
    /// Receive a discovery message from some discovery module.
    Discovery(ODiscoveryMessage),
}

/// Builder for constructing an `UberModule`.
pub struct UberModuleBuilder {
    // TODO: Remove these bounds when something like https://github.com/rust-lang/rust/pull/45047 lands
    discovery: Vec<
        Box<
            dyn DiscoveryTrait<
                SinkItem = IDiscoveryMessage,
                SinkError = DiscoveryError,
                Item = ODiscoveryMessage,
                Error = DiscoveryError,
            >,
        >,
    >,
    ext_builder: Option<ExtendedMessageBuilder>,
}

impl UberModuleBuilder {
    /// Create a new `UberModuleBuilder`.
    pub fn new() -> UberModuleBuilder {
        UberModuleBuilder {
            discovery: Vec::new(),
            ext_builder: None,
        }
    }

    /// Specifies the given builder that all modules will add to when sending an extended message to a peer.
    ///
    /// This message will only be sent when the extension bit from the handshake it set. Note that if a builder
    /// is not given and a peer with the extension bit set connects, we will NOT send any extended message.
    pub fn with_extended_builder(
        mut self,
        builder: Option<ExtendedMessageBuilder>,
    ) -> UberModuleBuilder {
        self.ext_builder = builder;
        self
    }

    /// Add the given discovery module to the list of discovery modules.
    pub fn with_discovery_module<T>(mut self, module: T) -> UberModuleBuilder
    where
        T: ExtendedListener
            + Sink<SinkItem = IDiscoveryMessage, SinkError = DiscoveryError>
            + Stream<Item = ODiscoveryMessage, Error = DiscoveryError>
            + Send
            + 'static,
    {
        self.discovery.push(Box::new(module)
            as Box<
                dyn DiscoveryTrait<
                    SinkItem = IDiscoveryMessage,
                    SinkError = DiscoveryError,
                    Item = ODiscoveryMessage,
                    Error = DiscoveryError,
                >,
            >);
        self
    }

    /// Build an `UberModule` based on the current builder.
    pub fn build(self) -> UberModule {
        UberModule::from_builder(self)
    }
}

//----------------------------------------------------------------------//

// TODO: Try to get generic is_ready trait into futures-rs
trait IsReady {
    fn is_ready(&self) -> bool;
}

impl<T> IsReady for AsyncSink<T> {
    fn is_ready(&self) -> bool {
        self.is_ready()
    }
}

impl<T> IsReady for Async<T> {
    fn is_ready(&self) -> bool {
        self.is_ready()
    }
}

//----------------------------------------------------------------------//

/// Module for multiplexing messages across zero or more other modules.
pub struct UberModule {
    discovery: Vec<
        Box<
            dyn DiscoveryTrait<
                SinkItem = IDiscoveryMessage,
                SinkError = DiscoveryError,
                Item = ODiscoveryMessage,
                Error = DiscoveryError,
            >,
        >,
    >,
    extended: Option<ExtendedModule>,
    last_sink_state: Option<ModuleState>,
    last_stream_state: Option<ModuleState>,
}

#[derive(Debug, Copy, Clone)]
enum ModuleState {
    Extended,
    Discovery(usize),
}

impl UberModule {
    /// Create an `UberModule` from the given `UberModuleBuilder`.
    pub fn from_builder(builder: UberModuleBuilder) -> UberModule {
        UberModule {
            discovery: builder.discovery,
            extended: builder
                .ext_builder
                .map(|builder| ExtendedModule::new(builder)),
            last_sink_state: None,
            last_stream_state: None,
        }
    }

    /// Get the next state after the given state, return Some(next_state) or None if the given state was the last state.
    ///
    /// We return the next state regardless of the message we are processing at the time. So if we dont recognize the tuple of
    /// next state and message, we ignore it. This makes the implemenation a lot easier as we dont have to do an exhaustive match
    /// on all possible states and messages, as only a subset will be valid.
    fn next_state(&self, state: Option<ModuleState>) -> Option<ModuleState> {
        match state {
            None => {
                if self.extended.is_some() {
                    Some(ModuleState::Extended)
                } else if !self.discovery.is_empty() {
                    Some(ModuleState::Discovery(0))
                } else {
                    None
                }
            }
            Some(ModuleState::Extended) => {
                if !self.discovery.is_empty() {
                    Some(ModuleState::Discovery(0))
                } else {
                    None
                }
            }
            Some(ModuleState::Discovery(index)) => {
                if index + 1 < self.discovery.len() {
                    Some(ModuleState::Discovery(index + 1))
                } else {
                    None
                }
            }
        }
    }

    /// Loop over all states until we finish, or hit an error.
    ///
    /// Takes care of saving/reseting states if we hit an error/finish.
    fn loop_states<G, A, L, R, E>(
        &mut self,
        is_sink: bool,
        init: Result<R, E>,
        get_next_state: G,
        assign_state: A,
        logic: L,
    ) -> Result<R, E>
    where
        G: Fn(&UberModule) -> Option<ModuleState>,
        A: Fn(&mut UberModule, Option<ModuleState>),
        L: Fn(&mut UberModule, ModuleState) -> Result<R, E>,
        R: IsReady,
    {
        let is_stream = !is_sink;
        let mut result = init;

        loop{
            //根据已完成节点找到下一个节点
            let mut opt_next_state = get_next_state(self);

            if opt_next_state.is_none() {
                assign_state(self, None);
                return  result;
            }

            result = logic(self, opt_next_state.unwrap());

            //判断是否继续执行下一个节点
            //是sink 或者  是stream但返回值不可读，则继续执行
            let should_continue = result
                .as_ref()
                .map(|var| (is_sink && var.is_ready()) || (is_stream && !var.is_ready()))
                .unwrap_or(false);

            // If we dont need to return to the user because of this error, mark it as done
            if should_continue {
                //继续执行则存储已完成状态
                assign_state(self, opt_next_state);
            }else {
                return result;
            }

        }
    }

    /// Run the start_send logic for the current module for the given message.
    fn start_sink_state(&mut self, message: &IUberMessage) -> StartSend<(), UberError> {
        self.loop_states(
            true,
            Ok(AsyncSink::Ready),
            |uber| uber.next_state(uber.last_sink_state),
            |uber, state| {
                uber.last_sink_state = state;
            },
            |uber, state| match (message,state) {

                (&IUberMessage::Control(ref control),ModuleState::Extended) => {
                    let d_modules = &mut uber.discovery[..];
                    uber.extended
                        .as_mut()
                        .map(|ext_module| {
                            ext_module.process_message(
                                IExtendedMessage::Control(control.clone()),
                                d_modules,
                            );
                            Ok(AsyncSink::Ready)
                        })
                        .unwrap_or(Ok(AsyncSink::Ready))
                }

                (&IUberMessage::Control(ref control),ModuleState::Discovery(index)) => uber
                    .discovery[index]
                    .start_send(IDiscoveryMessage::Control(control.clone()))
                    .map(|var| var.map(|_| ()))
                    .map_err(|err| err.into()),


                (&IUberMessage::Extended(ref extended),ModuleState::Extended,) => {
                    let d_modules = &mut uber.discovery[..];
                    uber.extended
                        .as_mut()
                        .map(|ext_module| {
                            ext_module.process_message(extended.clone(), d_modules);

                            Ok(AsyncSink::Ready)
                        })
                        .unwrap_or(Ok(AsyncSink::Ready))
                }

                (&IUberMessage::Discovery(ref discovery),ModuleState::Discovery(index) ) => uber
                    .discovery[index]
                    .start_send(discovery.clone())
                    .map(|var| var.map(|_| ()))
                    .map_err(|err| err.into()),

                _ => Ok(AsyncSink::Ready),
            },
        )
    }

    fn poll_sink_state(&mut self) -> Poll<(), UberError> {
        self.loop_states(
            true,
            Ok(Async::Ready(())),
            |uber| uber.next_state(uber.last_sink_state),
            |uber, state| {
                uber.last_sink_state = state;
            },
            |uber, state|
                match state {
                    ModuleState::Extended => Ok(Async::Ready(())),

                    ModuleState::Discovery(index) => uber.discovery[index]
                        .poll_complete()
                        .map_err(|err| err.into()),
            },
        )
    }

    fn poll_stream_state(&mut self) -> Poll<Option<OUberMessage>, UberError> {
        self.loop_states(
            false,
            Ok(Async::NotReady),
            |uber| uber.next_state(uber.last_stream_state),
            |uber, state| {
                uber.last_stream_state = state;
            },
            |uber, state|
                match state {
                    ModuleState::Extended => uber
                        .extended
                        .as_mut()
                        .map(|ext_module| {
                            ext_module
                                .poll()
                                .map(|async_opt_message| {
                                    async_opt_message.map(|opt_message| {
                                        opt_message.map(|message| OUberMessage::Extended(message))
                                    })
                                })
                                .map_err(|err| err.into())
                        })
                        .unwrap_or(Ok(Async::Ready(None))),

                        ModuleState::Discovery(index) => uber.discovery[index]
                            .poll()
                            .map(|async_opt_message| {
                                async_opt_message.map(|opt_message| {
                                    opt_message.map(|message| OUberMessage::Discovery(message))
                                })
                            })
                            .map_err(|err| err.into()),
            },
        )
    }
}

impl Sink for UberModule {
    type SinkItem = IUberMessage;
    type SinkError = UberError;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        // Currently we dont return NotReady from the module directly, so no saving our task state here
        self.start_sink_state(&item).map(|var| var.map(|_| item))
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        self.poll_sink_state()
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        // unimplemented!()
        Ok(futures::Async::Ready(()))
    }
}

impl Stream for UberModule {
    type Item = OUberMessage;
    type Error = UberError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let result = self.poll_stream_state();

        result
    }
}
