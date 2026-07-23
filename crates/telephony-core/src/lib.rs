use serde::{Deserialize, Serialize};
use std::{fmt, net::Ipv4Addr};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub server: String,
    pub local_address: String,
    pub sip_port: u16,
    pub rtp_port_start: u16,
    pub rtp_port_end: u16,
    pub extension: String,
    #[serde(skip_serializing)]
    pub password: String,
}

impl AccountConfig {
    /// Validate the RT-500KI-specific constraints used by the first release.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidConfig`] when an address, credential,
    /// extension, or fixed port constraint is invalid.
    pub fn validate(&self) -> Result<(), EngineError> {
        let server = self
            .server
            .parse::<Ipv4Addr>()
            .map_err(|_| EngineError::InvalidConfig("server must be an IPv4 address".into()))?;
        let local_address = self.local_address.parse::<Ipv4Addr>().map_err(|_| {
            EngineError::InvalidConfig("local address must be an IPv4 address".into())
        })?;
        if !server.is_private() || !local_address.is_private() || server == local_address {
            return Err(EngineError::InvalidConfig(
                "server and local address must be distinct private LAN addresses".into(),
            ));
        }
        if self.extension != "3" {
            return Err(EngineError::InvalidConfig(
                "the initial release only supports extension 3".into(),
            ));
        }
        if self.sip_port == 0 {
            return Err(EngineError::InvalidConfig(
                "SIP port must be non-zero".into(),
            ));
        }
        if self.rtp_port_start > self.rtp_port_end
            || self.rtp_port_start % 2 != 0
            || self.rtp_port_end - self.rtp_port_start < 9
        {
            return Err(EngineError::InvalidConfig(
                "RTP range must begin on an even port and contain at least 10 ports".into(),
            ));
        }
        if self.password.chars().count() < 32 {
            return Err(EngineError::InvalidConfig(
                "password must contain at least 32 characters".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistrationState {
    #[default]
    Stopped,
    Registering,
    Registered,
    RetryWaiting,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallPhase {
    Ringing,
    Dialing,
    Connecting,
    Active,
    Ending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSnapshot {
    pub id: u32,
    pub peer: String,
    pub direction: CallDirection,
    pub phase: CallPhase,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneSnapshot {
    pub registration: RegistrationState,
    pub call: Option<CallSnapshot>,
    pub muted: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    RegistrationChanged(RegistrationState),
    IncomingCall { id: u32, peer: String },
    OutgoingCall { id: u32, peer: String },
    CallConnecting { id: u32 },
    CallActive { id: u32 },
    CallEnded { id: u32, reason: String },
    Muted(bool),
    Failed(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EngineError {
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("operation is not valid in the current state: {0}")]
    InvalidState(String),
    #[error("telephony backend is unavailable: {0}")]
    BackendUnavailable(String),
    #[error("telephony backend failed: {0}")]
    Backend(String),
}

pub trait TelephonyEngine: Send {
    /// Start the backend and register the account.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is invalid or registration
    /// cannot be started.
    fn start(&mut self, config: &AccountConfig) -> Result<(), EngineError>;

    /// Unregister and stop the backend.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend cannot shut down cleanly.
    fn stop(&mut self) -> Result<(), EngineError>;

    /// Start an outgoing call.
    ///
    /// # Errors
    ///
    /// Returns an error when unregistered, busy, or the number is invalid.
    fn dial(&mut self, number: &str) -> Result<(), EngineError>;

    /// Answer the current incoming call.
    ///
    /// # Errors
    ///
    /// Returns an error when no incoming call exists or signaling fails.
    fn answer(&mut self) -> Result<(), EngineError>;

    /// Reject the current incoming call with `486 Busy Here`.
    ///
    /// # Errors
    ///
    /// Returns an error when no incoming call exists or signaling fails.
    fn reject(&mut self) -> Result<(), EngineError>;

    /// End the current call.
    ///
    /// # Errors
    ///
    /// Returns an error when no call exists or signaling fails.
    fn hang_up(&mut self) -> Result<(), EngineError>;

    /// Send one DTMF digit on the active call.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid digit, inactive call, or media failure.
    fn send_dtmf(&mut self, digit: char) -> Result<(), EngineError>;

    /// Enable or disable microphone mute while preserving the media session.
    ///
    /// # Errors
    ///
    /// Returns an error when no active call exists or media routing fails.
    fn set_muted(&mut self, muted: bool) -> Result<(), EngineError>;

    /// Return a secret-free snapshot suitable for presentation over IPC.
    fn snapshot(&self) -> PhoneSnapshot;
}

#[derive(Debug, Default)]
pub struct StateMachine {
    snapshot: PhoneSnapshot,
}

impl StateMachine {
    #[must_use]
    pub fn snapshot(&self) -> PhoneSnapshot {
        self.snapshot.clone()
    }

    /// Apply an event while enforcing single-call state transitions.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidState`] if the event does not apply to
    /// the current call or registration state.
    pub fn apply(&mut self, event: EngineEvent) -> Result<(), EngineError> {
        match event {
            EngineEvent::RegistrationChanged(state) => {
                self.snapshot.registration = state;
                if state == RegistrationState::Registered {
                    self.snapshot.last_error = None;
                }
            }
            EngineEvent::IncomingCall { id, peer } => {
                self.ensure_idle("receive a call")?;
                self.snapshot.call = Some(CallSnapshot {
                    id,
                    peer,
                    direction: CallDirection::Incoming,
                    phase: CallPhase::Ringing,
                });
                self.snapshot.muted = false;
            }
            EngineEvent::OutgoingCall { id, peer } => {
                self.ensure_idle("start a call")?;
                if self.snapshot.registration != RegistrationState::Registered {
                    return Err(EngineError::InvalidState(
                        "outgoing calls require registration".into(),
                    ));
                }
                self.snapshot.call = Some(CallSnapshot {
                    id,
                    peer,
                    direction: CallDirection::Outgoing,
                    phase: CallPhase::Dialing,
                });
                self.snapshot.muted = false;
            }
            EngineEvent::CallConnecting { id } => {
                self.call_mut(id)?.phase = CallPhase::Connecting;
            }
            EngineEvent::CallActive { id } => {
                self.call_mut(id)?.phase = CallPhase::Active;
            }
            EngineEvent::CallEnded { id, .. } => {
                self.call_mut(id)?;
                self.snapshot.call = None;
                self.snapshot.muted = false;
            }
            EngineEvent::Muted(muted) => {
                if self.snapshot.call.as_ref().map(|call| call.phase) != Some(CallPhase::Active) {
                    return Err(EngineError::InvalidState(
                        "mute requires an active call".into(),
                    ));
                }
                self.snapshot.muted = muted;
            }
            EngineEvent::Failed(message) => {
                self.snapshot.last_error = Some(message);
            }
        }
        Ok(())
    }

    fn ensure_idle(&self, operation: &str) -> Result<(), EngineError> {
        if self.snapshot.call.is_some() {
            return Err(EngineError::InvalidState(format!(
                "cannot {operation} while another call exists"
            )));
        }
        Ok(())
    }

    fn call_mut(&mut self, id: u32) -> Result<&mut CallSnapshot, EngineError> {
        match self.snapshot.call.as_mut() {
            Some(call) if call.id == id => Ok(call),
            _ => Err(EngineError::InvalidState(format!(
                "call {id} is not current"
            ))),
        }
    }
}

impl fmt::Display for RegistrationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_call_requires_registration() {
        let mut state = StateMachine::default();
        let error = state
            .apply(EngineEvent::OutgoingCall {
                id: 1,
                peer: "0312345678".into(),
            })
            .unwrap_err();
        assert!(matches!(error, EngineError::InvalidState(_)));
    }

    #[test]
    fn complete_incoming_call_returns_to_idle() {
        let mut state = StateMachine::default();
        state
            .apply(EngineEvent::IncomingCall {
                id: 7,
                peer: "anonymous".into(),
            })
            .unwrap();
        state.apply(EngineEvent::CallActive { id: 7 }).unwrap();
        state.apply(EngineEvent::Muted(true)).unwrap();
        state
            .apply(EngineEvent::CallEnded {
                id: 7,
                reason: "remote hangup".into(),
            })
            .unwrap();
        assert_eq!(state.snapshot().call, None);
        assert!(!state.snapshot().muted);
    }

    #[test]
    fn rejects_second_simultaneous_call() {
        let mut state = StateMachine::default();
        state
            .apply(EngineEvent::IncomingCall {
                id: 1,
                peer: "1".into(),
            })
            .unwrap();
        let error = state
            .apply(EngineEvent::IncomingCall {
                id: 2,
                peer: "2".into(),
            })
            .unwrap_err();
        assert!(matches!(error, EngineError::InvalidState(_)));
    }

    #[test]
    fn validates_rt500_port_range() {
        let config = AccountConfig {
            server: "192.168.1.1".into(),
            local_address: "192.168.1.252".into(),
            sip_port: 5060,
            rtp_port_start: 40_001,
            rtp_port_end: 40_009,
            extension: "3".into(),
            password: "0123456789abcdef0123456789abcdef".into(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_non_lan_bind_address() {
        let config = AccountConfig {
            server: "192.168.1.1".into(),
            local_address: "100.100.1.1".into(),
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            extension: "3".into(),
            password: "0123456789abcdef0123456789abcdef".into(),
        };
        assert!(config.validate().is_err());
    }
}
