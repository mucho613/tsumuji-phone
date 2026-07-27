#[cfg(not(feature = "native-pjsip"))]
use telephony_core::StateMachine;
use telephony_core::{AccountConfig, EngineError, PhoneSnapshot, TelephonyEngine};

#[cfg(feature = "native-pjsip")]
mod native;

#[cfg(all(feature = "native-pjsip", target_os = "android"))]
pub use native::set_android_java_vm;

#[must_use]
pub const fn backend_description() -> &'static str {
    if cfg!(feature = "native-pjsip") {
        "PJSIP 2.17 C adapter (native-pjsip enabled)"
    } else {
        "PJSIP adapter present; native-pjsip disabled"
    }
}

#[derive(Debug, Default)]
pub struct PjsipEngine {
    #[cfg(not(feature = "native-pjsip"))]
    state: StateMachine,
    #[cfg(feature = "native-pjsip")]
    native: native::NativeEngine,
}

impl PjsipEngine {
    /// Retry registration for an already-started native account.
    ///
    /// # Errors
    ///
    /// Returns an error when native PJSIP is unavailable or not running.
    pub fn reregister(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            self.native.reregister()
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Err(EngineError::BackendUnavailable(
                "native PJSIP support is disabled".into(),
            ))
        }
    }

    /// Stop the local SIP stack without removing a push-enabled registrar binding.
    ///
    /// # Errors
    ///
    /// Returns an error when native PJSIP cannot shut down cleanly.
    pub fn stop_dormant(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            self.native.stop_dormant()
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            self.state
                .apply(telephony_core::EngineEvent::RegistrationChanged(
                    telephony_core::RegistrationState::Stopped,
                ))
        }
    }
}

#[allow(clippy::needless_return)]
impl TelephonyEngine for PjsipEngine {
    fn start(&mut self, config: &AccountConfig) -> Result<(), EngineError> {
        config.validate()?;
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.start(config);
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Err(EngineError::BackendUnavailable(
                "rebuild with --features telephony-pjsip/native-pjsip".into(),
            ))
        }
    }

    fn stop(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.stop();
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Ok(())
        }
    }

    fn dial(&mut self, number: &str) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.dial(number);
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            let _ = number;
            Err(EngineError::BackendUnavailable(
                "native PJSIP support is disabled".into(),
            ))
        }
    }

    fn answer(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.answer();
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Err(native_disabled())
        }
    }

    fn reject(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.reject();
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Err(native_disabled())
        }
    }

    fn hang_up(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.hang_up();
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            Err(native_disabled())
        }
    }

    fn send_dtmf(&mut self, digit: char) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.send_dtmf(digit);
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            let _ = digit;
            Err(EngineError::BackendUnavailable(
                "native PJSIP support is disabled".into(),
            ))
        }
    }

    fn set_muted(&mut self, muted: bool) -> Result<(), EngineError> {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.set_muted(muted);
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            let _ = muted;
            Err(EngineError::BackendUnavailable(
                "native PJSIP support is disabled".into(),
            ))
        }
    }

    fn snapshot(&self) -> PhoneSnapshot {
        #[cfg(feature = "native-pjsip")]
        {
            return self.native.snapshot();
        }
        #[cfg(not(feature = "native-pjsip"))]
        {
            self.state.snapshot()
        }
    }
}

#[cfg(not(feature = "native-pjsip"))]
fn native_disabled() -> EngineError {
    EngineError::BackendUnavailable("native PJSIP support is disabled".into())
}
