use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr::{self, NonNull};
use std::sync::Mutex;
use telephony_core::{
    AccountConfig, EngineError, EngineEvent, PhoneSnapshot, RegistrationState, StateMachine,
};

#[repr(C)]
struct TpEngine {
    _private: [u8; 0],
}

#[repr(C)]
struct TpConfig {
    server: *const c_char,
    local_address: *const c_char,
    user_id: *const c_char,
    password: *const c_char,
    push_device_id: *const c_char,
    sip_port: u16,
    rtp_port_start: u16,
    rtp_port_end: u16,
}

#[repr(C)]
struct TpEvent {
    kind: c_int,
    call_id: i32,
    peer: *const c_char,
    message: *const c_char,
}

type TpEventCallback = unsafe extern "C" fn(*mut c_void, *const TpEvent);

unsafe extern "C" {
    #[cfg(target_os = "android")]
    fn tp_android_set_jvm(java_vm: *mut c_void);
    fn tp_engine_start(
        config: *const TpConfig,
        callback: TpEventCallback,
        user_data: *mut c_void,
        out_engine: *mut *mut TpEngine,
    ) -> c_int;
    fn tp_engine_stop(engine: *mut TpEngine) -> c_int;
    fn tp_engine_reregister(engine: *mut TpEngine) -> c_int;
    fn tp_engine_dial(engine: *mut TpEngine, number: *const c_char) -> c_int;
    fn tp_engine_answer(engine: *mut TpEngine) -> c_int;
    fn tp_engine_reject(engine: *mut TpEngine) -> c_int;
    fn tp_engine_hang_up(engine: *mut TpEngine) -> c_int;
    fn tp_engine_send_dtmf(engine: *mut TpEngine, digit: c_char) -> c_int;
    fn tp_engine_set_muted(engine: *mut TpEngine, muted: c_int) -> c_int;
    fn tp_last_error(engine: *const TpEngine) -> *const c_char;
}

#[cfg(target_os = "android")]
pub fn set_android_java_vm(java_vm: usize) {
    unsafe {
        tp_android_set_jvm(java_vm as *mut c_void);
    }
}

#[derive(Debug, Default)]
struct CallbackState {
    state: Mutex<StateMachine>,
}

#[derive(Debug)]
pub struct NativeEngine {
    handle: Option<NonNull<TpEngine>>,
    callback_state: Box<CallbackState>,
}

// PJSUA invokes callbacks from its worker threads. CallbackState is synchronized,
// and the adapter supports one process-global engine.
unsafe impl Send for NativeEngine {}

impl Default for NativeEngine {
    fn default() -> Self {
        Self {
            handle: None,
            callback_state: Box::new(CallbackState::default()),
        }
    }
}

impl NativeEngine {
    pub fn start(&mut self, config: &AccountConfig) -> Result<(), EngineError> {
        if self.handle.is_some() {
            return Err(EngineError::InvalidState(
                "PJSIP engine is already running".into(),
            ));
        }
        let server = to_c_string(&config.server)?;
        let local_address = to_c_string(&config.local_address)?;
        let user_id = to_c_string(&config.user_id)?;
        let password = to_c_string(&config.password)?;
        let push_device_id = to_c_string(config.push_device_id.as_deref().unwrap_or(""))?;
        let native_config = TpConfig {
            server: server.as_ptr(),
            local_address: local_address.as_ptr(),
            user_id: user_id.as_ptr(),
            password: password.as_ptr(),
            push_device_id: push_device_id.as_ptr(),
            sip_port: config.sip_port,
            rtp_port_start: config.rtp_port_start,
            rtp_port_end: config.rtp_port_end,
        };
        let mut handle = ptr::null_mut();
        let user_data = (&raw mut *self.callback_state)
            .cast::<CallbackState>()
            .cast::<c_void>();
        // The adapter copies all config strings before this call returns.
        let status = unsafe {
            tp_engine_start(
                &raw const native_config,
                on_event,
                user_data,
                &raw mut handle,
            )
        };
        if status != 0 {
            return Err(EngineError::Backend(format!(
                "PJSIP initialization failed with status {status}"
            )));
        }
        self.handle = NonNull::new(handle);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), EngineError> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let status = unsafe { tp_engine_stop(handle.as_ptr()) };
        let result = if status == 0 {
            Ok(())
        } else {
            Err(EngineError::Backend(format!(
                "PJSIP shutdown failed with status {status}"
            )))
        };
        if result.is_ok() {
            self.apply(EngineEvent::RegistrationChanged(RegistrationState::Stopped));
        }
        result
    }

    pub fn reregister(&mut self) -> Result<(), EngineError> {
        let result = self.call(|handle| unsafe { tp_engine_reregister(handle) });
        if result.is_ok() {
            self.apply(EngineEvent::RegistrationChanged(
                RegistrationState::Registering,
            ));
        }
        result
    }

    pub fn dial(&mut self, number: &str) -> Result<(), EngineError> {
        if number.is_empty()
            || !number
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, '*' | '#'))
        {
            return Err(EngineError::InvalidConfig(
                "phone number contains unsupported characters".into(),
            ));
        }
        let number = to_c_string(number)?;
        self.call(|handle| unsafe { tp_engine_dial(handle, number.as_ptr()) })
    }

    pub fn answer(&mut self) -> Result<(), EngineError> {
        self.call(|handle| unsafe { tp_engine_answer(handle) })
    }

    pub fn reject(&mut self) -> Result<(), EngineError> {
        self.call(|handle| unsafe { tp_engine_reject(handle) })
    }

    pub fn hang_up(&mut self) -> Result<(), EngineError> {
        self.call(|handle| unsafe { tp_engine_hang_up(handle) })
    }

    pub fn send_dtmf(&mut self, digit: char) -> Result<(), EngineError> {
        if !matches!(digit, '0'..='9' | '*' | '#') {
            return Err(EngineError::InvalidConfig("invalid DTMF digit".into()));
        }
        self.call(|handle| unsafe { tp_engine_send_dtmf(handle, digit as c_char) })
    }

    pub fn set_muted(&mut self, muted: bool) -> Result<(), EngineError> {
        let result = self.call(|handle| unsafe { tp_engine_set_muted(handle, c_int::from(muted)) });
        if result.is_ok() {
            self.apply(EngineEvent::Muted(muted));
        }
        result
    }

    #[must_use]
    pub fn snapshot(&self) -> PhoneSnapshot {
        self.callback_state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
    }

    fn call(&mut self, operation: impl FnOnce(*mut TpEngine) -> c_int) -> Result<(), EngineError> {
        let handle = self
            .handle
            .ok_or_else(|| EngineError::InvalidState("PJSIP engine is not running".into()))?;
        status_result(operation(handle.as_ptr()), handle.as_ptr())
    }

    fn apply(&self, event: EngineEvent) {
        let mut state = self
            .callback_state
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(error) = state.apply(event) {
            let _ = state.apply(EngineEvent::Failed(error.to_string()));
        }
    }
}

impl Drop for NativeEngine {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

unsafe extern "C" fn on_event(user_data: *mut c_void, event: *const TpEvent) {
    if user_data.is_null() || event.is_null() {
        return;
    }
    // Both pointers are owned by the Rust engine/C adapter for this callback's duration.
    let callback_state = unsafe { &*user_data.cast::<CallbackState>() };
    let event = unsafe { &*event };
    let peer = c_string_lossy(event.peer);
    let message = c_string_lossy(event.message);
    let registration_failure = (event.kind == 3).then(|| format!("registration failed: {message}"));
    let call_id = u32::try_from(event.call_id).unwrap_or_default();
    let mapped = match event.kind {
        1 => EngineEvent::RegistrationChanged(RegistrationState::Registering),
        2 => EngineEvent::RegistrationChanged(RegistrationState::Registered),
        3 => EngineEvent::RegistrationChanged(RegistrationState::Failed),
        4 => EngineEvent::IncomingCall { id: call_id, peer },
        5 => EngineEvent::OutgoingCall { id: call_id, peer },
        6 => EngineEvent::CallConnecting { id: call_id },
        7 => EngineEvent::CallActive { id: call_id },
        8 => EngineEvent::CallEnded {
            id: call_id,
            reason: message,
        },
        _ => EngineEvent::Failed(message),
    };
    let mut state = callback_state
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Err(error) = state.apply(mapped) {
        let _ = state.apply(EngineEvent::Failed(error.to_string()));
    } else if let Some(error) = registration_failure {
        let _ = state.apply(EngineEvent::Failed(error));
    }
}

fn to_c_string(value: &str) -> Result<CString, EngineError> {
    CString::new(value).map_err(|_| EngineError::InvalidConfig("value contains a NUL byte".into()))
}

fn c_string_lossy(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}

fn status_result(status: c_int, handle: *mut TpEngine) -> Result<(), EngineError> {
    if status == 0 {
        return Ok(());
    }
    let detail = c_string_lossy(unsafe { tp_last_error(handle) });
    Err(EngineError::Backend(if detail.is_empty() {
        format!("PJSIP operation failed with status {status}")
    } else {
        detail
    }))
}
