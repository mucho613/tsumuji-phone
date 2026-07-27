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
    pub user_id: String,
    #[serde(skip_serializing)]
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_device_id: Option<String>,
}

impl AccountConfig {
    /// Validate the SIP account and local media constraints used by the app.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidConfig`] when an address, credential,
    /// user ID, or fixed port constraint is invalid.
    pub fn validate(&self) -> Result<(), EngineError> {
        if !is_valid_server(&self.server) {
            return Err(EngineError::InvalidConfig(
                "server must be an IPv4 address or DNS hostname".into(),
            ));
        }
        let local_address = self.local_address.parse::<Ipv4Addr>().map_err(|_| {
            EngineError::InvalidConfig("local address must be an IPv4 address".into())
        })?;
        if !is_private_or_tailscale(local_address) {
            return Err(EngineError::InvalidConfig(
                "local address must be a private LAN or Tailscale IPv4 address".into(),
            ));
        }
        if self
            .server
            .parse::<Ipv4Addr>()
            .is_ok_and(|server| server == local_address)
        {
            return Err(EngineError::InvalidConfig(
                "server and local address must be distinct".into(),
            ));
        }
        if !is_valid_user_id(&self.user_id) {
            return Err(EngineError::InvalidConfig(
                "user ID must contain 1 to 32 ASCII letters, digits, or -._~".into(),
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
        if !(1..=128).contains(&self.password.len())
            || !self.password.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(EngineError::InvalidConfig(
                "password must contain 1 to 128 printable ASCII characters without spaces".into(),
            ));
        }
        if self.push_device_id.as_ref().is_some_and(|device_id| {
            device_id.len() != 22
                || !device_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }) {
            return Err(EngineError::InvalidConfig(
                "push device ID must be a 128-bit Base64URL value".into(),
            ));
        }
        Ok(())
    }
}

fn is_private_or_tailscale(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    address.is_private() || (octets[0] == 100 && (64..=127).contains(&octets[1]))
}

fn is_valid_user_id(user_id: &str) -> bool {
    (1..=32).contains(&user_id.len())
        && user_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn is_valid_server(server: &str) -> bool {
    if let Ok(address) = server.parse::<Ipv4Addr>() {
        return !address.is_unspecified()
            && !address.is_multicast()
            && address != Ipv4Addr::BROADCAST;
    }

    let hostname = server.strip_suffix('.').unwrap_or(server);
    !hostname.is_empty()
        && hostname.len() <= 253
        && hostname.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistrationState {
    #[default]
    Stopped,
    PlatformReady,
    PushReady,
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
    pub last_call_reason: Option<String>,
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
                if matches!(
                    state,
                    RegistrationState::Registering | RegistrationState::Registered
                ) {
                    self.snapshot.last_error = None;
                }
            }
            EngineEvent::IncomingCall { id, peer } => {
                self.ensure_idle("receive a call")?;
                self.snapshot.last_call_reason = None;
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
                self.snapshot.last_call_reason = None;
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
            EngineEvent::CallEnded { id, reason } => {
                self.call_mut(id)?;
                self.snapshot.call = None;
                self.snapshot.muted = false;
                self.snapshot.last_call_reason = Some(reason);
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
    fn a_new_registration_attempt_clears_the_previous_error() {
        let mut state = StateMachine::default();
        state
            .apply(EngineEvent::RegistrationChanged(RegistrationState::Failed))
            .unwrap();
        state
            .apply(EngineEvent::Failed("registration failed".into()))
            .unwrap();

        state
            .apply(EngineEvent::RegistrationChanged(
                RegistrationState::Registering,
            ))
            .unwrap();

        assert_eq!(
            state.snapshot().registration,
            RegistrationState::Registering
        );
        assert_eq!(state.snapshot().last_error, None);
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
        assert_eq!(
            state.snapshot().last_call_reason.as_deref(),
            Some("remote hangup")
        );
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
    fn validates_rtp_port_range() {
        let config = AccountConfig {
            server: "asterisk.example.ts.net".into(),
            local_address: "100.100.1.1".into(),
            sip_port: 5060,
            rtp_port_start: 40_001,
            rtp_port_end: 40_009,
            user_id: "ts-line3".into(),
            password: "secret".into(),
            push_device_id: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_non_lan_or_tailscale_bind_address() {
        let config = AccountConfig {
            server: "asterisk.example.ts.net".into(),
            local_address: "8.8.8.8".into(),
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            user_id: "ts-line3".into(),
            password: "secret".into(),
            push_device_id: None,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn accepts_asterisk_over_tailscale() {
        let config = AccountConfig {
            server: "asterisk.example.ts.net".into(),
            local_address: "100.100.1.5".into(),
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            user_id: "ts-line3".into(),
            password: "a1b2c3!@".into(),
            push_device_id: None,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_user_id_and_password() {
        let mut config = AccountConfig {
            server: "asterisk.example.ts.net".into(),
            local_address: "100.100.1.5".into(),
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            user_id: "phone@3".into(),
            password: "a".into(),
            push_device_id: None,
        };
        assert!(config.validate().is_err());

        config.user_id = "user id".into();
        assert!(config.validate().is_err());

        config.user_id = "ts-line3".into();
        config.password = String::new();
        assert!(config.validate().is_err());

        config.password = "contains space".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_server_hostname() {
        let config = AccountConfig {
            server: "-asterisk.example.ts.net".into(),
            local_address: "100.100.1.5".into(),
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            user_id: "ts-line3".into(),
            password: "secret".into(),
            push_device_id: None,
        };
        assert!(config.validate().is_err());
    }
}
