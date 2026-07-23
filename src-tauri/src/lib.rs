#![allow(clippy::needless_pass_by_value)]

use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State,
};
use telephony_core::{AccountConfig, PhoneSnapshot, TelephonyEngine};
use telephony_pjsip::PjsipEngine;

const KEYRING_SERVICE: &str = "app.hikari-phone.local";
const KEYRING_USER: &str = "extension-3";
const BLOCKED_NUMBERS: [&str; 3] = ["110", "118", "119"];

struct Controller {
    engine: Mutex<PjsipEngine>,
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            engine: Mutex::new(PjsipEngine::default()),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartRequest {
    server: String,
    local_address: String,
    password: Option<String>,
}

fn backend_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn is_blocked_number(number: &str) -> bool {
    BLOCKED_NUMBERS.contains(&number)
}

#[tauri::command]
fn get_phone_state(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let engine = controller.engine.lock().map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn start_waiting(
    request: StartRequest,
    controller: State<'_, Controller>,
) -> Result<PhoneSnapshot, String> {
    let password = match request.password {
        Some(password) if !password.is_empty() => {
            keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
                .map_err(backend_error)?
                .set_password(&password)
                .map_err(backend_error)?;
            password
        }
        _ => keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(backend_error)?
            .get_password()
            .map_err(|_| "ひかり電話のパスワードを入力してください".to_owned())?,
    };
    let config = AccountConfig {
        server: request.server,
        local_address: request.local_address,
        sip_port: 5060,
        rtp_port_start: 40_000,
        rtp_port_end: 40_009,
        extension: "3".into(),
        password,
    };
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.start(&config).map_err(backend_error)?;
    Ok(engine.snapshot())
}

#[tauri::command]
fn stop_waiting(controller: State<'_, Controller>) -> Result<PhoneSnapshot, String> {
    let mut engine = controller.engine.lock().map_err(backend_error)?;
    engine.stop().map_err(backend_error)?;
    Ok(engine.snapshot())
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
fn reset_credentials() -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(backend_error)?
        .delete_credential()
        .map_err(backend_error)
}

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
    tauri::Builder::default()
        .manage(Controller::default())
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
        })
        .invoke_handler(tauri::generate_handler![
            get_phone_state,
            start_waiting,
            stop_waiting,
            dial,
            answer,
            reject,
            hang_up,
            send_dtmf,
            set_muted,
            reset_credentials
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::is_blocked_number;

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
}
