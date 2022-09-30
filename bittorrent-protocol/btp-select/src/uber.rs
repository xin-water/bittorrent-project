use btp_peer::messages::builders::ExtendedMessageBuilder;
use crate::discovery::error::DiscoveryError;
use crate::discovery::{IDiscoveryMessage, ODiscoveryMessage, Run};
use crate::error::UberError;
use crate::extended::ExtendedModule;
use crate::{ControlMessage, ExtendedListener, IExtendedMessage, OExtendedMessage};

pub trait DiscoveryTrait: ExtendedListener + Run + Send + Sync{
}

impl<T> DiscoveryTrait for T
    where T: ExtendedListener + Run +Send + Sync {
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
    ext_builder: Option<ExtendedMessageBuilder>,
    discovery: Vec<Box<dyn DiscoveryTrait>>,
}

impl UberModuleBuilder {
    /// Create a new `UberModuleBuilder`.
    pub fn new() -> UberModuleBuilder {
        UberModuleBuilder {
            ext_builder: None,
            discovery: Vec::new(),
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
        T: ExtendedListener + Run + Send + Sync +'static
    {
        self.discovery.push(
            Box::new(module)  as Box<dyn DiscoveryTrait>
        );
        self
    }

    /// Build an `UberModule` based on the current builder.
    pub fn build(self) -> UberModule {
        UberModule::from_builder(self)
    }
}

//----------------------------------------------------------------------//

/// Module for multiplexing messages across zero or more other modules.
pub struct UberModule {
    extended: Option<ExtendedModule>,
    discovery: Vec<Box<dyn DiscoveryTrait>>,
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
            extended: builder
                .ext_builder
                .map(|builder| ExtendedModule::new(builder)),
            discovery: builder.discovery,
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
                    //不是最后一个 Discovery时，序号加1
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
    fn loop_states<R, E, G, A, L>(
        &mut self,
        is_sink: bool,
        init: Result<Option<R>, E>,
        get_next_state: G,
        assign_state: A,
        logic: L,
    ) -> Result<Option<R>, E>
        where
            G: Fn(&UberModule) -> Option<ModuleState>,
            A: Fn(&mut UberModule, Option<ModuleState>),
            L: Fn(&mut UberModule, ModuleState) -> Result<Option<R>, E>,
    {
        let is_stream = !is_sink;
        let mut result = init;

        loop {
            let mut opt_next_state = get_next_state(self);

            if opt_next_state.is_none() {
                assign_state(self, None);
                return result;
            }

            result = logic(self, opt_next_state.unwrap());

            let should_continue:bool=result
                .as_ref()
                .map(|var|{
                    is_sink  ||  is_stream && var.is_none()
                })
                .unwrap_or(false);

            // If we dont need to return to the user because of this error, mark it as done
            if should_continue {
                assign_state(self, opt_next_state);
            }else {
                return result;
            }

        }
    }

    /// Run the start_send logic for the current module for the given message.
    fn start_sink_state(&mut self, message: &IUberMessage) -> Result<Option<()>, UberError> {
        self.loop_states(
            true,
            Ok(None),
            |uber| uber.next_state(uber.last_sink_state),
            |uber, state| {
                uber.last_sink_state = state;
            },
            |uber, state|
                match (state, message) {
                    (ModuleState::Extended, &IUberMessage::Control(ref control)) => {
                        let d_modules = &mut uber.discovery[..];
                        uber.extended
                            .as_mut()
                            .map(|ext_module| {
                                ext_module.process_message(IExtendedMessage::Control(control.clone()), d_modules);
                                Ok(None)
                            })
                            .unwrap_or(Ok(None))
                    }
                    (ModuleState::Extended, &IUberMessage::Extended(ref extended)) => {
                        let d_modules = &mut uber.discovery[..];
                        uber.extended
                            .as_mut()
                            .map(|ext_module| {
                                ext_module.process_message(extended.clone(), d_modules);
                                Ok(None)
                            })
                            .unwrap_or(Ok(None))
                    }
                    (ModuleState::Discovery(index), &IUberMessage::Control(ref control)) => uber
                        .discovery[index]
                        .send(IDiscoveryMessage::Control(control.clone()))
                        .map(|var| var.map(|_| ()))
                        .map_err(|err| err.into()),
                    (ModuleState::Discovery(index), &IUberMessage::Discovery(ref discovery)) => uber
                        .discovery[index]
                        .send(discovery.clone())
                        .map(|var| var.map(|_| ()))
                        .map_err(|err| err.into()),
                    _ => Ok(None),
            },
        )
    }

    fn poll_stream_state(&mut self) -> Result<Option<OUberMessage>, UberError> {
        self.loop_states(
            false,
            Ok(None),
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
                                .map(|opt_message| {
                                    opt_message.map(|message| OUberMessage::Extended(message))
                                })
                                .map_err(|err| err.into())
                        })
                        .unwrap_or(Ok(None)),

                    ModuleState::Discovery(index) => uber
                        .discovery[index]
                        .poll()
                        .map(|opt_message| {
                            opt_message
                                .map(|message| Some(OUberMessage::Discovery(message)))
                                .map_err(|err| err.into())
                        })
                        .unwrap_or(Ok(None)),
                }
        )
    }
}

impl UberModule {
   pub fn send(&mut self, item: IUberMessage) -> Result<Option<()>, UberError> {
        // Currently we dont return NotReady from the module directly, so no saving our task state here
        self.start_sink_state(&item)
    }
}

impl UberModule {
   pub fn poll(&mut self) -> Result<Option<OUberMessage>, UberError> {
        let result = self.poll_stream_state();

        result
    }
}
