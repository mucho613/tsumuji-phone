use std::{
    fs::{self, File},
    num::NonZero,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait},
    DeviceId,
};
use rodio::{
    source::{SineWave, Zero},
    Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source,
};
use tauri::{menu::CheckMenuItem, AppHandle, Manager, UserAttentionType, Wry};
use telephony_core::{CallDirection, CallPhase, PhoneSnapshot, TelephonyEngine};
use telephony_pjsip::PjsipEngine;

const SETTINGS_FILE: &str = "desktop-alerts.json";
const RINGTONE_DIRECTORY: &str = "ringtones";
const PREVIEW_DURATION: Duration = Duration::from_secs(10);
const MONITOR_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DesktopAlertMode {
    Normal,
    Quiet,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
struct PersistedSettings {
    mode: DesktopAlertMode,
    volume: u8,
    ringtone_file_name: Option<String>,
    ringtone_path: Option<PathBuf>,
    output_device_id: Option<String>,
    output_device_name: Option<String>,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            mode: DesktopAlertMode::Normal,
            volume: 100,
            ringtone_file_name: None,
            ringtone_path: None,
            output_device_id: None,
            output_device_name: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RuntimeStatus {
    previewing: bool,
    playback_error: Option<String>,
    device_fallback: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopAlertSettings {
    mode: DesktopAlertMode,
    volume: u8,
    ringtone_file_name: Option<String>,
    output_device_id: Option<String>,
    output_device_name: Option<String>,
    previewing: bool,
    playback_error: Option<String>,
    device_fallback: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopAudioDevice {
    id: String,
    name: String,
    is_default: bool,
}

#[derive(Debug, Clone)]
struct PlaybackRequest {
    custom_path: Option<PathBuf>,
    device_id: Option<String>,
    volume: u8,
    preview: bool,
}

enum AudioCommand {
    Start {
        request: PlaybackRequest,
        result: Sender<Result<(bool, Option<String>), String>>,
    },
    Stop,
    SetVolume(u8),
    Shutdown,
}

struct ActivePlayback {
    _stream: MixerDeviceSink,
    player: Player,
    request: PlaybackRequest,
    device_error: Receiver<()>,
}

pub struct DesktopAlerts {
    settings: Arc<Mutex<PersistedSettings>>,
    runtime: Arc<Mutex<RuntimeStatus>>,
    settings_path: PathBuf,
    ringtone_directory: PathBuf,
    audio_tx: Sender<AudioCommand>,
}

pub struct DesktopTrayMenu {
    pub normal: CheckMenuItem<Wry>,
    pub quiet: CheckMenuItem<Wry>,
}

impl DesktopAlerts {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let config_directory = app.path().app_config_dir().map_err(display_error)?;
        let data_directory = app.path().app_data_dir().map_err(display_error)?;
        fs::create_dir_all(&config_directory).map_err(display_error)?;
        let ringtone_directory = data_directory.join(RINGTONE_DIRECTORY);
        fs::create_dir_all(&ringtone_directory).map_err(display_error)?;
        let settings_path = config_directory.join(SETTINGS_FILE);
        let settings = load_settings(&settings_path);
        let runtime = Arc::new(Mutex::new(RuntimeStatus::default()));
        let (audio_tx, audio_rx) = mpsc::channel();
        let audio_runtime = Arc::clone(&runtime);
        thread::Builder::new()
            .name("desktop-ringtone".into())
            .spawn(move || audio_worker(audio_rx, audio_runtime))
            .map_err(display_error)?;
        Ok(Self {
            settings: Arc::new(Mutex::new(settings)),
            runtime,
            settings_path,
            ringtone_directory,
            audio_tx,
        })
    }

    pub fn snapshot(&self) -> Result<DesktopAlertSettings, String> {
        let settings = self.settings.lock().map_err(display_error)?.clone();
        let runtime = self.runtime.lock().map_err(display_error)?.clone();
        Ok(DesktopAlertSettings {
            mode: settings.mode,
            volume: settings.volume,
            ringtone_file_name: settings.ringtone_file_name,
            output_device_id: settings.output_device_id,
            output_device_name: settings.output_device_name,
            previewing: runtime.previewing,
            playback_error: runtime.playback_error,
            device_fallback: runtime.device_fallback,
        })
    }

    pub fn mode(&self) -> DesktopAlertMode {
        self.settings
            .lock()
            .map_or(DesktopAlertMode::Normal, |settings| settings.mode)
    }

    pub fn set_mode(
        &self,
        app: &AppHandle,
        mode: DesktopAlertMode,
        phone: &PhoneSnapshot,
    ) -> Result<DesktopAlertSettings, String> {
        {
            let mut settings = self.settings.lock().map_err(display_error)?;
            settings.mode = mode;
            self.persist(&settings)?;
        }
        sync_tray_mode(app, mode);
        if is_incoming_ringing(phone) {
            match mode {
                DesktopAlertMode::Normal => {
                    show_incoming_window(app);
                    self.start_ringing();
                }
                DesktopAlertMode::Quiet => {
                    self.stop_audio();
                    cancel_attention(app);
                }
            }
        }
        self.snapshot()
    }

    pub fn set_volume(&self, volume: u8) -> Result<DesktopAlertSettings, String> {
        let volume = volume.min(100);
        {
            let mut settings = self.settings.lock().map_err(display_error)?;
            settings.volume = volume;
            self.persist(&settings)?;
        }
        let _ = self.audio_tx.send(AudioCommand::SetVolume(volume));
        self.snapshot()
    }

    pub fn set_output_device(
        &self,
        device_id: Option<String>,
    ) -> Result<DesktopAlertSettings, String> {
        let device_name = match device_id.as_deref() {
            Some(id) => Some(
                output_devices()?
                    .into_iter()
                    .find(|device| device.id == id)
                    .ok_or_else(|| "選択した着信音デバイスを利用できません".to_string())?
                    .name,
            ),
            None => None,
        };
        let previewing = self.runtime.lock().map_err(display_error)?.previewing;
        {
            let mut settings = self.settings.lock().map_err(display_error)?;
            settings.output_device_id = device_id;
            settings.output_device_name = device_name;
            self.persist(&settings)?;
        }
        if previewing {
            self.start_preview()?;
        }
        self.snapshot()
    }

    pub fn choose_ringtone(&self) -> Result<DesktopAlertSettings, String> {
        let Some(source_path) = rfd::FileDialog::new()
            .set_title("着信音を選択")
            .add_filter("音声ファイル", &["wav", "mp3", "ogg", "flac"])
            .pick_file()
        else {
            return self.snapshot();
        };
        let extension = supported_extension(&source_path)?;
        validate_audio_file(&source_path)?;
        let temporary_path = self.ringtone_directory.join("ringtone-import.tmp");
        fs::copy(&source_path, &temporary_path)
            .map_err(|error| format!("着信音をアプリへコピーできませんでした: {error}"))?;
        if let Err(error) = validate_audio_file(&temporary_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }

        let destination = self
            .ringtone_directory
            .join(format!("custom-ringtone.{extension}"));
        if destination.exists() {
            fs::remove_file(&destination).map_err(display_error)?;
        }
        fs::rename(&temporary_path, &destination).map_err(display_error)?;
        let original_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "着信音のファイル名を読み取れません".to_string())?
            .to_string();

        let previous_path = {
            let mut settings = self.settings.lock().map_err(display_error)?;
            let previous = settings.ringtone_path.clone();
            settings.ringtone_file_name = Some(original_name);
            settings.ringtone_path = Some(destination.clone());
            self.persist(&settings)?;
            previous
        };
        if let Some(previous_path) = previous_path {
            if previous_path != destination && previous_path.starts_with(&self.ringtone_directory) {
                let _ = fs::remove_file(previous_path);
            }
        }
        self.snapshot()
    }

    pub fn reset_ringtone(&self) -> Result<DesktopAlertSettings, String> {
        let previous_path = {
            let mut settings = self.settings.lock().map_err(display_error)?;
            let previous = settings.ringtone_path.take();
            settings.ringtone_file_name = None;
            self.persist(&settings)?;
            previous
        };
        if let Some(previous_path) = previous_path {
            if previous_path.starts_with(&self.ringtone_directory) {
                let _ = fs::remove_file(previous_path);
            }
        }
        self.snapshot()
    }

    pub fn start_preview(&self) -> Result<DesktopAlertSettings, String> {
        let request = self.playback_request(true)?;
        {
            let mut runtime = self.runtime.lock().map_err(display_error)?;
            runtime.previewing = true;
            runtime.playback_error = None;
        }
        if let Err(error) = self.start_audio(request) {
            let mut runtime = self.runtime.lock().map_err(display_error)?;
            runtime.previewing = false;
            runtime.playback_error = Some(error.clone());
            return Err(error);
        }
        self.snapshot()
    }

    pub fn stop_preview(&self) -> Result<DesktopAlertSettings, String> {
        self.stop_audio();
        self.snapshot()
    }

    fn start_ringing(&self) {
        self.stop_audio();
        let Ok(request) = self.playback_request(false) else {
            return;
        };
        if let Err(error) = self.start_audio(request) {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.playback_error = Some(format!("着信音を再生できませんでした: {error}"));
            }
        }
    }

    fn start_audio(&self, request: PlaybackRequest) -> Result<(), String> {
        let (result_tx, result_rx) = mpsc::channel();
        self.audio_tx
            .send(AudioCommand::Start {
                request,
                result: result_tx,
            })
            .map_err(display_error)?;
        let (used_fallback, playback_warning) =
            result_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| "着信音デバイスの開始がタイムアウトしました".to_string())??;
        let mut runtime = self.runtime.lock().map_err(display_error)?;
        runtime.device_fallback = used_fallback;
        runtime.playback_error = playback_warning.or_else(|| {
            used_fallback.then(|| "選択したデバイスを利用できないため、システム既定を使用中".into())
        });
        Ok(())
    }

    fn stop_audio(&self) {
        let _ = self.audio_tx.send(AudioCommand::Stop);
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.previewing = false;
            runtime.device_fallback = false;
        }
    }

    fn playback_request(&self, preview: bool) -> Result<PlaybackRequest, String> {
        let settings = self.settings.lock().map_err(display_error)?;
        Ok(PlaybackRequest {
            custom_path: settings.ringtone_path.clone(),
            device_id: settings.output_device_id.clone(),
            volume: settings.volume,
            preview,
        })
    }

    fn persist(&self, settings: &PersistedSettings) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(settings).map_err(display_error)?;
        fs::write(&self.settings_path, contents).map_err(display_error)
    }
}

impl Drop for DesktopAlerts {
    fn drop(&mut self) {
        let _ = self.audio_tx.send(AudioCommand::Shutdown);
    }
}

pub fn start_monitor(app: AppHandle, engine: Arc<Mutex<PjsipEngine>>, alerts: Arc<DesktopAlerts>) {
    thread::spawn(move || {
        let mut ringing_call_id = None;
        loop {
            thread::sleep(MONITOR_INTERVAL);
            let snapshot = match engine.lock() {
                Ok(engine) => engine.snapshot(),
                Err(_) => continue,
            };
            let current = snapshot
                .call
                .as_ref()
                .filter(|call| {
                    call.direction == CallDirection::Incoming && call.phase == CallPhase::Ringing
                })
                .map(|call| call.id);

            match (ringing_call_id, current) {
                (None, Some(call_id)) => {
                    alerts.stop_audio();
                    if alerts.mode() == DesktopAlertMode::Normal {
                        show_incoming_window(&app);
                        alerts.start_ringing();
                    }
                    ringing_call_id = Some(call_id);
                }
                (Some(_), None) => {
                    alerts.stop_audio();
                    cancel_attention(&app);
                    ringing_call_id = None;
                }
                (Some(previous), Some(current_id)) if previous != current_id => {
                    alerts.stop_audio();
                    if alerts.mode() == DesktopAlertMode::Normal {
                        show_incoming_window(&app);
                        alerts.start_ringing();
                    }
                    ringing_call_id = Some(current_id);
                }
                _ => {}
            }

            if ringing_call_id.is_some()
                && app
                    .get_webview_window("main")
                    .and_then(|window| window.is_focused().ok())
                    == Some(true)
            {
                cancel_attention(&app);
            }
        }
    });
}

pub fn sync_tray_mode(app: &AppHandle, mode: DesktopAlertMode) {
    if let Some(menu) = app.try_state::<DesktopTrayMenu>() {
        let _ = menu.normal.set_checked(mode == DesktopAlertMode::Normal);
        let _ = menu.quiet.set_checked(mode == DesktopAlertMode::Quiet);
    }
}

fn show_incoming_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let was_focused = window.is_focused().unwrap_or(false);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if !was_focused {
            let _ = window.request_user_attention(Some(UserAttentionType::Critical));
        }
    }
}

fn cancel_attention(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.request_user_attention(None);
    }
}

fn is_incoming_ringing(snapshot: &PhoneSnapshot) -> bool {
    snapshot.call.as_ref().is_some_and(|call| {
        call.direction == CallDirection::Incoming && call.phase == CallPhase::Ringing
    })
}

fn output_devices() -> Result<Vec<DesktopAudioDevice>, String> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let mut devices = host
        .output_devices()
        .map_err(|error| format!("着信音デバイスを取得できませんでした: {error}"))?
        .filter_map(|device| {
            let id = device.id().ok()?.to_string();
            #[allow(deprecated)]
            let name = device.name().ok()?;
            Some(DesktopAudioDevice {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            })
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(devices)
}

pub fn list_output_devices() -> Result<Vec<DesktopAudioDevice>, String> {
    output_devices()
}

fn load_settings(path: &Path) -> PersistedSettings {
    fs::read(path)
        .ok()
        .and_then(|contents| serde_json::from_slice(&contents).ok())
        .unwrap_or_default()
}

fn supported_extension(path: &Path) -> Result<&str, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "WAV、MP3、OGG、FLACの音声ファイルを選択してください".to_string())?;
    match extension.as_str() {
        "wav" | "mp3" | "ogg" | "flac" => Ok(match extension.as_str() {
            "wav" => "wav",
            "mp3" => "mp3",
            "ogg" => "ogg",
            _ => "flac",
        }),
        _ => Err("WAV、MP3、OGG、FLACの音声ファイルを選択してください".into()),
    }
}

fn validate_audio_file(path: &Path) -> Result<(), String> {
    let file =
        File::open(path).map_err(|error| format!("着信音ファイルを開けませんでした: {error}"))?;
    Decoder::try_from(file)
        .map(|_| ())
        .map_err(|error| format!("着信音ファイルを読み取れませんでした: {error}"))
}

fn audio_worker(receiver: Receiver<AudioCommand>, runtime: Arc<Mutex<RuntimeStatus>>) {
    let mut playback: Option<ActivePlayback> = None;
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some(command) = command else {
            if playback
                .as_ref()
                .is_some_and(|active| active.request.preview && active.player.empty())
            {
                playback = None;
                if let Ok(mut status) = runtime.lock() {
                    status.previewing = false;
                }
                continue;
            }
            if playback
                .as_ref()
                .is_some_and(|active| active.device_error.try_recv().is_ok())
            {
                let mut request = playback.take().expect("playback checked above").request;
                request.device_id = None;
                match start_playback(&request) {
                    Ok((active, _)) => {
                        playback = Some(active);
                        if let Ok(mut status) = runtime.lock() {
                            status.device_fallback = true;
                            status.playback_error =
                                Some("着信音デバイスが切断されたため、システム既定を使用中".into());
                        }
                    }
                    Err(error) => {
                        if let Ok(mut status) = runtime.lock() {
                            status.previewing = false;
                            status.playback_error =
                                Some(format!("着信音を再生できませんでした: {error}"));
                        }
                    }
                }
            }
            continue;
        };
        match command {
            AudioCommand::Start { request, result } => {
                playback = None;
                let response = start_playback(&request);
                match response {
                    Ok((active, fallback)) => {
                        playback = Some(active);
                        let _ = result.send(Ok((fallback, None)));
                    }
                    Err(error) if request.custom_path.is_some() => {
                        let fallback_request = PlaybackRequest {
                            custom_path: None,
                            ..request
                        };
                        match start_playback(&fallback_request) {
                            Ok((active, device_fallback)) => {
                                playback = Some(active);
                                let _ = result.send(Ok((
                                    device_fallback,
                                    Some(format!(
                                        "選択した着信音を再生できなかったため、既定の着信音を使用中: {error}"
                                    )),
                                )));
                            }
                            Err(default_error) => {
                                let _ = result.send(Err(format!(
                                    "{error}; 既定の着信音も再生できませんでした: {default_error}"
                                )));
                            }
                        }
                    }
                    Err(error) => {
                        let _ = result.send(Err(error));
                    }
                }
            }
            AudioCommand::Stop => {
                if let Some(active) = playback.take() {
                    active.player.stop();
                }
            }
            AudioCommand::SetVolume(volume) => {
                if let Some(active) = playback.as_ref() {
                    active.player.set_volume(f32::from(volume) / 100.0);
                }
            }
            AudioCommand::Shutdown => break,
        }
    }
}

fn start_playback(request: &PlaybackRequest) -> Result<(ActivePlayback, bool), String> {
    let (stream, used_fallback, device_error) = open_output_stream(request.device_id.as_deref())?;
    let player = Player::connect_new(stream.mixer());
    player.set_volume(f32::from(request.volume) / 100.0);

    if let Some(path) = request.custom_path.as_ref() {
        let file = File::open(path).map_err(display_error)?;
        let source = Decoder::new_looped(file).map_err(display_error)?;
        if request.preview {
            player.append(source.take_duration(PREVIEW_DURATION));
        } else {
            player.append(source);
        }
    } else {
        let source = default_chime().repeat_infinite();
        if request.preview {
            player.append(source.take_duration(PREVIEW_DURATION));
        } else {
            player.append(source);
        }
    }

    Ok((
        ActivePlayback {
            _stream: stream,
            player,
            request: request.clone(),
            device_error,
        },
        used_fallback,
    ))
}

fn open_output_stream(
    device_id: Option<&str>,
) -> Result<(MixerDeviceSink, bool, Receiver<()>), String> {
    let (error_tx, error_rx) = mpsc::channel();
    let open_device = |device| {
        let error_tx = error_tx.clone();
        DeviceSinkBuilder::from_device(device)
            .map(|builder| {
                builder.with_error_callback(move |error| {
                    if matches!(
                        error,
                        cpal::StreamError::DeviceNotAvailable
                            | cpal::StreamError::StreamInvalidated
                    ) {
                        let _ = error_tx.send(());
                    }
                })
            })
            .and_then(|builder| builder.open_sink_or_fallback())
    };
    if let Some(device_id) = device_id {
        let selected = DeviceId::from_str(device_id)
            .ok()
            .and_then(|id| cpal::default_host().device_by_id(&id));
        if let Some(device) = selected {
            if let Ok(stream) = open_device(device) {
                return Ok((stream, false, error_rx));
            }
        }
        let default_device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| "システム既定の着信音デバイスがありません".to_string())?;
        return open_device(default_device)
            .map(|stream| (stream, true, error_rx))
            .map_err(display_error);
    }
    let default_device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "システム既定の着信音デバイスがありません".to_string())?;
    open_device(default_device)
        .map(|stream| (stream, false, error_rx))
        .map_err(display_error)
}

fn default_chime() -> impl Source<Item = f32> + Clone {
    let note = |frequency| {
        SineWave::new(frequency)
            .take_duration(Duration::from_millis(260))
            .fade_in(Duration::from_millis(20))
            .fade_out(Duration::from_millis(70))
            .amplify(0.22)
    };
    note(523.25)
        .mix(note(659.25).delay(Duration::from_millis(260)))
        .mix(note(523.25).delay(Duration::from_millis(700)))
        .mix(note(659.25).delay(Duration::from_millis(960)))
        .mix(Zero::new_samples(
            NonZero::new(1).expect("one channel"),
            NonZero::new(48_000).expect("valid sample rate"),
            48_000 * 3_220 / 1_000,
        ))
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::{default_chime, is_incoming_ringing, supported_extension, DesktopAlertMode};
    use std::path::Path;
    use telephony_core::{
        CallDirection, CallPhase, CallSnapshot, PhoneSnapshot, RegistrationState,
    };

    #[test]
    fn accepts_only_supported_ringtone_extensions() {
        for extension in ["wav", "MP3", "ogg", "FlAc"] {
            assert!(supported_extension(Path::new(&format!("ringtone.{extension}"))).is_ok());
        }
        for extension in ["aac", "opus", "txt"] {
            assert!(supported_extension(Path::new(&format!("ringtone.{extension}"))).is_err());
        }
    }

    #[test]
    fn detects_only_incoming_ringing_calls() {
        let snapshot = |direction, phase| PhoneSnapshot {
            registration: RegistrationState::Registered,
            call: Some(CallSnapshot {
                id: 1,
                peer: "sip:4@asterisk".into(),
                direction,
                phase,
                active_elapsed_seconds: None,
            }),
            ..PhoneSnapshot::default()
        };

        assert!(is_incoming_ringing(&snapshot(
            CallDirection::Incoming,
            CallPhase::Ringing,
        )));
        assert!(!is_incoming_ringing(&snapshot(
            CallDirection::Incoming,
            CallPhase::Active,
        )));
        assert!(!is_incoming_ringing(&snapshot(
            CallDirection::Outgoing,
            CallPhase::Ringing,
        )));
    }

    #[test]
    fn default_chime_has_the_expected_loop_length() {
        assert_eq!(default_chime().count(), 48_000 * 3_220 / 1_000);
    }

    #[test]
    fn alert_modes_are_distinct() {
        assert_ne!(DesktopAlertMode::Normal, DesktopAlertMode::Quiet);
    }
}
