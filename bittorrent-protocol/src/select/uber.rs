use crate::peer::messages::builders::ExtendedMessageBuilder;
use crate::select::ut_metadata::error::UtMetadataError;
use crate::select::ut_metadata::{IUtMetadataMessage, OUtMetadataMessage, UtMetadataModule};
use crate::select::error::UberError;
use crate::select::extended::ExtendedModule;
use crate::select::{ControlMessage, ExtendedListener, IExtendedMessage, OExtendedMessage};

trait DiscoveryTrait: ExtendedListener {
}

impl<T> DiscoveryTrait for T
    where T: ExtendedListener  {
}

/// Enumeration of uber messages that can be sent to the uber module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IUberMessage {
    /// Broadcast a control message out to all modules.
    Control(ControlMessage),
    /// Send an extended message to the extended module.
    Extended(IExtendedMessage),
    /// Send a ut_metadata message to all ut_metadata modules.
    Ut_Metadata(IUtMetadataMessage),
}

/// Enumeration of uber messages that can be received from the uber module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OUberMessage {
    /// Receive an extended message from the extended module.
    Extended(OExtendedMessage),
    /// Receive a ut_metadata message from some ut_metadata module.
    Ut_Metadata(OUtMetadataMessage),
}

/// Builder for constructing an `UberModule`.
pub struct UberModuleBuilder {
    ext_builder: Option<ExtendedMessageBuilder>,
    ut_metadata: Option<UtMetadataModule>,
}

impl UberModuleBuilder {
    /// Create a new `UberModuleBuilder`.
    pub fn new() -> UberModuleBuilder {
        UberModuleBuilder {
            ext_builder: None,
            ut_metadata: None,
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

    /// Add the given ut_metadata module to the list of ut_metadata modules.
    pub fn with_ut_metadata_module(mut self, module: UtMetadataModule) -> UberModuleBuilder {
        self.ut_metadata =Some(module);
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
    ut_metadata: Option<UtMetadataModule>,
    last_sink_state: Option<ModuleState>,
    last_stream_state: Option<ModuleState>,
}

#[derive(Debug, Copy, Clone)]
enum ModuleState {
    Extended,
    UtMetadata,
}

impl UberModule {
    /// Create an `UberModule` from the given `UberModuleBuilder`.
    pub fn from_builder(builder: UberModuleBuilder) -> UberModule {
        UberModule {
            extended: builder
                .ext_builder
                .map(|builder| ExtendedModule::new(builder)),
            ut_metadata: builder.ut_metadata,
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
                } else if self.ut_metadata.is_some() {
                    Some(ModuleState::UtMetadata)
                } else {
                    None
                }
            }
            Some(ModuleState::Extended) => {
                if self.ut_metadata.is_some() {
                    Some(ModuleState::UtMetadata)
                } else {
                    None
                }
            }
            Some(ModuleState::UtMetadata) => {
                    None
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
                match (message,state ) {
                    (&IUberMessage::Control(ref control),ModuleState::Extended) => {
                        let d_modules = uber
                            .ut_metadata
                            .as_mut()
                            .expect(" Get UtMetadataModule Fail");

                        uber.extended
                            .as_mut()
                            .map(|ext_module| {
                                ext_module.process_message(IExtendedMessage::Control(control.clone()), d_modules);
                                Ok(None)
                            })
                            .unwrap_or(Ok(None))
                    }

                    (&IUberMessage::Control(ref control),ModuleState::UtMetadata,) => uber
                        .ut_metadata
                        .as_mut()
                        .expect(" Get UtMetadataModule Fail")
                        .send(IUtMetadataMessage::Control(control.clone()))
                        .map(|var| var.map(|_| ()))
                        .map_err(|err| err.into()),

                    (&IUberMessage::Extended(ref extended),ModuleState::Extended) => {
                        let d_modules = uber
                            .ut_metadata
                            .as_mut()
                            .expect(" Get UtMetadataModule Fail");
                        uber.extended
                            .as_mut()
                            .map(|ext_module| {
                                ext_module.process_message(extended.clone(), d_modules);
                                Ok(None)
                            })
                            .unwrap_or(Ok(None))
                    }

                    ( &IUberMessage::Ut_Metadata(ref metadata_msg),ModuleState::UtMetadata) => uber
                        .ut_metadata
                        .as_mut()
                        .expect(" Get UtMetadataModule Fail")
                        .send(metadata_msg.clone())
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

                    ModuleState::UtMetadata => uber
                        .ut_metadata
                        .as_mut()
                        .expect(" Get UtMetadataModule Fail")
                        .poll()
                        .map(|opt_message| {
                            opt_message
                                .map(|message| Some(OUberMessage::Ut_Metadata(message)))
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
