#![allow(clippy::needless_pass_by_value)]

#[cfg(target_os = "android")]
mod android_platform;

use std::sync::{Arc, Mutex};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr},
};
use tauri::State;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use telephony_core::AccountConfig;
use telephony_core::{PhoneSnapshot, TelephonyEngine};
use telephony_pjsip::PjsipEngine;

#[cfg(any(target_os = "linux", target_os = "windows"))]
const KEYRING_SERVICE: &str = "app.tsumuji-phone.local";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const LEGACY_KEYRING_SERVICE: &str = "app.hikari-phone.local";
#[cfg(any(target_os = "linux", target_os = "windows"))]
const KEYRING_ACCOUNT: &str = "sip-account";
const BLOCKED_NUMBERS: [&str; 3] = ["110", "118", "119"];

struct Controller {
    engine: Arc<Mutex<PjsipEngine>>,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl Default for Controller {
    fn default() -> Self {
        Self {
            engine: Arc::new(Mutex::new(PjsipEngine::default())),
        }
    }
}

#[cfg(target_os = "android")]
impl Default for Controller {
    fn default() -> Self {
        Self {
            engine: android_platform::shared_engine(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    server: String,
    user_id: String,
    password: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairPushGatewayRequest {
    gateway_url: String,
    pairing_code: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
struct PlatformInfo {
    is_android: bool,
    tailscale_address: Option<String>,
    credential_stored: bool,
    saved_server: Option<String>,
    saved_user_id: Option<String>,
    notification_permission: String,
    full_screen_intent_permission: bool,
    push_fid_ready: bool,
    push_paired: bool,
    push_gateway_url: Option<String>,
    push_device_id: Option<String>,
    push_pairing_error: Option<String>,
    push_setup_error: Option<String>,
    last_push_result: Option<String>,
    last_push_received_at: Option<i64>,
    last_push_registered_at: Option<i64>,
    last_push_duration_millis: Option<i64>,
    last_push_error: Option<String>,
}

fn backend_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(any(target_os = "android", test))]
fn merge_android_phone_snapshot(
    mut engine_snapshot: PhoneSnapshot,
    platform_snapshot: PhoneSnapshot,
) -> PhoneSnapshot {
    engine_snapshot.registration = platform_snapshot.registration;
    engine_snapshot.last_error = platform_snapshot.last_error.or(engine_snapshot.last_error);
    engine_snapshot
}

fn is_blocked_number(number: &str) -> bool {
    BLOCKED_NUMBERS.contains(&number)
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn is_tailscale_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn select_tailscale_ipv4(
    addresses: impl IntoIterator<Item = Ipv4Addr>,
) -> Result<Ipv4Addr, String> {
    let addresses = addresses
        .into_iter()
        .filter(|address| is_tailscale_ipv4(*address))
        .collect::<BTreeSet<_>>();
    match addresses.len() {
        0 => Err(
            "TailscaleのIPv4アドレスを検出できません。Tailscaleへ接続してから再試行してください"
                .into(),
        ),
        1 => Ok(*addresses.first().expect("one Tailscale address")),
        _ => Err(
            "Tailscale IPv4アドレスを一意に特定できません。Tailscaleの接続状態を確認してください"
                .into(),
        ),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn detect_tailscale_ipv4() -> Result<Ipv4Addr, String> {
    let addresses = if_addrs::get_if_addrs()
        .map_err(|error| format!("Tailscale IPv4アドレスの検出に失敗しました: {error}"))?
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        });
    select_tailscale_ipv4(addresses)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn keyring_entry(service: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, KEYRING_ACCOUNT).map_err(backend_error)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn delete_credential_if_present(entry: &keyring::Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(backend_error(error)),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn delete_legacy_credential() -> Result<(), String> {
    delete_credential_if_present(&keyring_entry(LEGACY_KEYRING_SERVICE)?)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn load_or_migrate_password(credential: &keyring::Entry) -> Result<String, String> {
    match credential.get_password() {
        Ok(password) => {
            delete_legacy_credential()?;
            Ok(password)
        }
        Err(keyring::Error::NoEntry) => {
            let legacy = keyring_entry(LEGACY_KEYRING_SERVICE)?;
            let password = match legacy.get_password() {
                Ok(password) => password,
                Err(keyring::Error::NoEntry) => return Err("パスワードを入力してください".into()),
                Err(error) => return Err(backend_error(error)),
            };
            credential.set_password(&password).map_err(backend_error)?;
            delete_credential_if_present(&legacy)?;
            Ok(password)
        }
        Err(error) => Err(backend_error(error)),
    }
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn get_phone_state(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let engine = controller.engine.lock().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn get_phone_state(
    platform: State<'_, android_platform::AndroidPlatform>,
    controller: State<'_, Controller>,
) -> Result<PhoneSnapshot, String> {
    let platform_snapshot = platform.status()?.phone_snapshot();
    let engine = controller.engine.lock().map_err(backend_error)?;
    Ok(merge_android_phone_snapshot(
        engine.snapshot(),
        platform_snapshot,
    ))
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn get_platform_info() -> PlatformInfo {
    PlatformInfo {
        is_android: false,
        tailscale_address: detect_tailscale_ipv4()
            .ok()
            .map(|address| address.to_string()),
        credential_stored: false,
        saved_server: None,
        saved_user_id: None,
        notification_permission: "unavailable".into(),
        full_screen_intent_permission: false,
        push_fid_ready: false,
        push_paired: false,
        push_gateway_url: None,
        push_device_id: None,
        push_pairing_error: None,
        push_setup_error: None,
        last_push_result: None,
        last_push_received_at: None,
        last_push_registered_at: None,
        last_push_duration_millis: None,
        last_push_error: None,
    }
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn get_platform_info(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<PlatformInfo, String> {
    let status = platform.status()?;
    Ok(PlatformInfo {
        is_android: true,
        tailscale_address: status.tailscale_address,
        credential_stored: status.credential_stored,
        saved_server: status.saved_server,
        saved_user_id: status.saved_user_id,
        notification_permission: status.notification_permission,
        full_screen_intent_permission: status.full_screen_intent_permission,
        push_fid_ready: status.push_fid_ready,
        push_paired: status.push_paired,
        push_gateway_url: status.push_gateway_url,
        push_device_id: status.push_device_id,
        push_pairing_error: status.push_pairing_error,
        push_setup_error: status.push_setup_error,
        last_push_result: status.last_push_result,
        last_push_received_at: status.last_push_received_at,
        last_push_registered_at: status.last_push_registered_at,
        last_push_duration_millis: status.last_push_duration_millis,
        last_push_error: status.last_push_error,
    })
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn request_push_permission() -> Result<(), String> {
    Err("Push通知はAndroid版でのみ使用できます".into())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn request_push_permission(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<(), String> {
    platform.request_push_permission()
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn open_push_notification_settings() -> Result<(), String> {
    Err("Push通知はAndroid版でのみ使用できます".into())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn open_push_notification_settings(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<(), String> {
    platform.open_push_notification_settings()
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn open_full_screen_intent_settings() -> Result<(), String> {
    Err("全画面着信の設定はAndroid版でのみ使用できます".into())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn open_full_screen_intent_settings(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<(), String> {
    platform.open_full_screen_intent_settings()
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn pair_push_gateway(request: PairPushGatewayRequest) -> Result<(), String> {
    let _ = (request.gateway_url, request.pairing_code);
    Err("Push gatewayとのペアリングはAndroid版でのみ使用できます".into())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn pair_push_gateway(
    request: PairPushGatewayRequest,
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<(), String> {
    platform.pair_push_gateway(request.gateway_url, request.pairing_code)
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn start_waiting(
    request: StartRequest,
    controller: State<'_, Controller>,
) -> Result<PhoneSnapshot, String> {
    let credential = keyring_entry(KEYRING_SERVICE)?;
    let supplied_password = request.password.filter(|password| !password.is_empty());
    let password = match supplied_password.as_ref() {
        Some(password) => password.clone(),
        None => load_or_migrate_password(&credential)?,
    };
    let config = AccountConfig {
        server: request.server,
        local_address: detect_tailscale_ipv4()?.to_string(),
        sip_port: 5060,
        rtp_port_start: 40_000,
        rtp_port_end: 40_009,
        user_id: request.user_id,
        password,
        push_device_id: None,
    };
    config.validate().map_err(backend_error)?;
    if let Some(password) = supplied_password {
        credential.set_password(&password).map_err(backend_error)?;
        delete_legacy_credential()?;
    }
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.start(&config).map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn start_waiting(
    request: StartRequest,
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<PhoneSnapshot, String> {
    let StartRequest {
        server,
        user_id,
        password,
    } = request;
    platform
        .start_waiting(server, user_id, password)
        .map(android_platform::AndroidStatus::phone_snapshot)
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn stop_waiting(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.stop().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn stop_waiting(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<PhoneSnapshot, String> {
    platform
        .stop_waiting()
        .map(android_platform::AndroidStatus::phone_snapshot)
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn reconnect() -> Result<PhoneSnapshot, String> {
    Err("再接続はAndroid版でのみ使用できます".into())
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn reconnect(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<PhoneSnapshot, String> {
    platform
        .reconnect()
        .map(android_platform::AndroidStatus::phone_snapshot)
}

#[tauri::command]
fn dial(number: String, controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    if is_blocked_number(&number) {
        return Err("緊急・特番への発信はこのアプリで禁止されています".into());
    }
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.dial(&number).map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn answer(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.answer().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn reject(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.reject().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn hang_up(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.hang_up().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn send_dtmf(digit: char, controller: State<'_, Controller>) -> Result<(), String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.send_dtmf(digit).map_err(backend_error)
}

#[tauri::command]
fn set_muted(muted: bool, controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.set_muted(muted).map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn reset_credentials() -> Result<(), String> {
    delete_credential_if_present(&keyring_entry(KEYRING_SERVICE)?)?;
    delete_legacy_credential()
}

#[tauri::command]
#[cfg(target_os = "android")]
async fn reset_credentials(
    platform: State<'_, android_platform::AndroidPlatform>,
) -> Result<(), String> {
    platform.reset_credentials()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Start the Tauri application and its process-wide telephony controller.
///
/// # Panics
///
/// Panics if Tauri cannot create the application runtime or system tray.
pub fn run() {
    let controller = Controller::default();
    let builder = tauri::Builder::default().manage(controller);
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android_platform::init());
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "表示", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .expect("application icon"),
                )
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    builder
        .invoke_handler(tauri::generate_handler![
            get_phone_state,
            get_platform_info,
            start_waiting,
            stop_waiting,
            reconnect,
            dial,
            answer,
            reject,
            hang_up,
            send_dtmf,
            set_muted,
            reset_credentials,
            request_push_permission,
            open_push_notification_settings,
            open_full_screen_intent_settings,
            pair_push_gateway
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{is_blocked_number, merge_android_phone_snapshot, select_tailscale_ipv4};
    use std::net::Ipv4Addr;
    use telephony_core::{
        CallDirection, CallPhase, CallSnapshot, PhoneSnapshot, RegistrationState,
    };

    #[test]
    fn blocks_japanese_emergency_and_coast_guard_numbers() {
        for number in ["110", "118", "119"] {
            assert!(is_blocked_number(number));
        }
    }

    #[test]
    fn does_not_block_numbers_that_only_contain_an_emergency_sequence() {
        assert!(!is_blocked_number("0110"));
        assert!(!is_blocked_number("1190"));
    }

    #[test]
    fn selects_the_only_tailscale_ipv4_address() {
        let selected = select_tailscale_ipv4([
            Ipv4Addr::new(192, 168, 1, 2),
            Ipv4Addr::new(100, 100, 1, 5),
            Ipv4Addr::new(100, 100, 1, 5),
        ])
        .unwrap();

        assert_eq!(selected, Ipv4Addr::new(100, 100, 1, 5));
    }

    #[test]
    fn rejects_missing_or_ambiguous_tailscale_ipv4_addresses() {
        assert!(select_tailscale_ipv4([Ipv4Addr::new(192, 168, 1, 2)]).is_err());
        assert!(select_tailscale_ipv4([
            Ipv4Addr::new(100, 64, 0, 1),
            Ipv4Addr::new(100, 127, 255, 254),
        ])
        .is_err());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires exactly one connected Tailscale IPv4 address"]
    fn detects_the_connected_desktop_tailscale_address() {
        super::detect_tailscale_ipv4().unwrap();
    }

    #[test]
    fn android_status_refresh_preserves_active_call() {
        let call = CallSnapshot {
            id: 7,
            peer: "sip:117@asterisk".into(),
            direction: CallDirection::Outgoing,
            phase: CallPhase::Active,
        };
        let engine_snapshot = PhoneSnapshot {
            registration: RegistrationState::Registered,
            call: Some(call.clone()),
            muted: true,
            last_error: None,
            last_call_reason: None,
        };
        let platform_snapshot = PhoneSnapshot {
            registration: RegistrationState::Registered,
            ..PhoneSnapshot::default()
        };

        let merged = merge_android_phone_snapshot(engine_snapshot, platform_snapshot);

        assert_eq!(merged.call, Some(call));
        assert!(merged.muted);
    }
}
