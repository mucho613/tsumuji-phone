use jni::{
    objects::{JObject, JString},
    sys::{jboolean, jstring},
    JNIEnv,
};
use serde::{Deserialize, Serialize};
use std::{
    ptr,
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Wry,
};
use telephony_core::{AccountConfig, PhoneSnapshot, RegistrationState, TelephonyEngine};
use telephony_pjsip::PjsipEngine;

const PLUGIN_IDENTIFIER: &str = "app.tsumujiphone.local";
static ANDROID_ENGINE: OnceLock<Arc<Mutex<PjsipEngine>>> = OnceLock::new();

pub struct AndroidPlatform(PluginHandle<Wry>);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStatus {
    state: AndroidWaitingState,
    pub tailscale_address: Option<String>,
    pub credential_stored: bool,
    pub saved_server: Option<String>,
    pub saved_user_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum AndroidWaitingState {
    Stopped,
    PlatformReady,
    Registering,
    Registered,
    Error,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartWaitingPayload {
    server: String,
    user_id: String,
    password: Option<String>,
}

impl AndroidStatus {
    pub fn phone_snapshot(self) -> PhoneSnapshot {
        let registration = match self.state {
            AndroidWaitingState::Stopped => RegistrationState::Stopped,
            AndroidWaitingState::PlatformReady => RegistrationState::PlatformReady,
            AndroidWaitingState::Registering => RegistrationState::Registering,
            AndroidWaitingState::Registered => RegistrationState::Registered,
            AndroidWaitingState::Error => RegistrationState::Failed,
        };
        PhoneSnapshot {
            registration,
            last_error: self.last_error,
            ..PhoneSnapshot::default()
        }
    }
}

impl AndroidPlatform {
    pub fn status(&self) -> Result<AndroidStatus, String> {
        self.0
            .run_mobile_plugin("getStatus", ())
            .map_err(|error| error.to_string())
    }

    pub fn start_waiting(
        &self,
        server: String,
        user_id: String,
        password: Option<String>,
    ) -> Result<AndroidStatus, String> {
        self.0
            .run_mobile_plugin(
                "startWaiting",
                StartWaitingPayload {
                    server,
                    user_id,
                    password,
                },
            )
            .map_err(|error| error.to_string())
    }

    pub fn stop_waiting(&self) -> Result<AndroidStatus, String> {
        self.0
            .run_mobile_plugin("stopWaiting", ())
            .map_err(|error| error.to_string())
    }

    pub fn reconnect(&self) -> Result<AndroidStatus, String> {
        self.0
            .run_mobile_plugin("reconnect", ())
            .map_err(|error| error.to_string())
    }

    pub fn reset_credentials(&self) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<()>("resetCredentials", ())
            .map_err(|error| error.to_string())
    }
}

pub fn install_engine(engine: Arc<Mutex<PjsipEngine>>) {
    assert!(
        ANDROID_ENGINE.set(engine).is_ok(),
        "Android PJSIP engine was initialized more than once"
    );
}

fn with_engine<T>(
    operation: impl FnOnce(&mut PjsipEngine) -> Result<T, String>,
) -> Result<T, String> {
    let engine = ANDROID_ENGINE
        .get()
        .ok_or_else(|| "Android PJSIPエンジンが初期化されていません".to_string())?;
    let mut engine = engine.lock().map_err(|error| error.to_string())?;
    operation(&mut engine)
}

fn read_java_string(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String, String> {
    env.get_string(value)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

fn optional_java_string(env: JNIEnv<'_>, value: Option<String>) -> jstring {
    value.map_or(ptr::null_mut(), |value| {
        env.new_string(value)
            .map_or(ptr::null_mut(), JString::into_raw)
    })
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeStart(
    mut env: JNIEnv<'_>,
    _instance: JObject<'_>,
    server: JString<'_>,
    user_id: JString<'_>,
    password: JString<'_>,
    local_address: JString<'_>,
) -> jstring {
    let result = (|| {
        let java_vm = env.get_java_vm().map_err(|error| error.to_string())?;
        telephony_pjsip::set_android_java_vm(java_vm.get_java_vm_pointer() as usize);
        let config = AccountConfig {
            server: read_java_string(&mut env, &server)?,
            local_address: read_java_string(&mut env, &local_address)?,
            sip_port: 5060,
            rtp_port_start: 40_000,
            rtp_port_end: 40_009,
            user_id: read_java_string(&mut env, &user_id)?,
            password: read_java_string(&mut env, &password)?,
        };
        with_engine(|engine| engine.start(&config).map_err(|error| error.to_string()))
    })();
    optional_java_string(env, result.err())
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeStop(
    env: JNIEnv<'_>,
    _instance: JObject<'_>,
) -> jstring {
    let result = with_engine(|engine| engine.stop().map_err(|error| error.to_string()));
    optional_java_string(env, result.err())
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeReregister(
    env: JNIEnv<'_>,
    _instance: JObject<'_>,
) -> jstring {
    let result = with_engine(|engine| engine.reregister().map_err(|error| error.to_string()));
    optional_java_string(env, result.err())
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeHasActiveCall(
    _env: JNIEnv<'_>,
    _instance: JObject<'_>,
) -> jboolean {
    u8::from(with_engine(|engine| Ok(engine.snapshot().call.is_some())).unwrap_or(false))
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeRegistrationState(
    env: JNIEnv<'_>,
    _instance: JObject<'_>,
) -> jstring {
    let state = with_engine(|engine| {
        Ok(match engine.snapshot().registration {
            RegistrationState::Stopped => "stopped",
            RegistrationState::PlatformReady => "platformReady",
            RegistrationState::PushReady => "pushReady",
            RegistrationState::Registering => "registering",
            RegistrationState::Registered => "registered",
            RegistrationState::RetryWaiting => "retryWaiting",
            RegistrationState::Failed => "failed",
        }
        .to_string())
    });
    optional_java_string(env, Some(state.unwrap_or_else(|_| "failed".to_string())))
}

#[no_mangle]
pub extern "system" fn Java_app_tsumujiphone_local_NativeTelephony_nativeLastError(
    env: JNIEnv<'_>,
    _instance: JObject<'_>,
) -> jstring {
    let error = with_engine(|engine| Ok(engine.snapshot().last_error)).unwrap_or_else(Some);
    optional_java_string(env, error)
}

pub fn init() -> TauriPlugin<Wry> {
    Builder::new("android-platform")
        .setup(|app, api| {
            let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "TelephonyPlugin")?;
            app.manage(AndroidPlatform(handle));
            Ok(())
        })
        .build()
}
