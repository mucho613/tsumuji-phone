import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Bell,
  BellOff,
  Bluetooth,
  Cable,
  Cast,
  Check,
  Delete,
  ExternalLink,
  FileAudio,
  Headphones,
  Mic,
  MicOff,
  Phone,
  PhoneCall,
  PhoneOff,
  Play,
  RotateCcw,
  Speaker,
  Settings,
  ShieldAlert,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import "./App.css";

type RegistrationState =
  | "stopped"
  | "platformReady"
  | "pushReady"
  | "registering"
  | "registered"
  | "retryWaiting"
  | "failed";

type Call = {
  id: number;
  peer: string;
  direction: "incoming" | "outgoing";
  phase: "ringing" | "dialing" | "connecting" | "active" | "ending";
  activeElapsedSeconds: number | null;
};

type PhoneState = {
  registration: RegistrationState;
  call: Call | null;
  muted: boolean;
  lastError: string | null;
  lastCallReason: string | null;
};

const initialPhoneState: PhoneState = {
  registration: "stopped",
  call: null,
  muted: false,
  lastError: null,
  lastCallReason: null,
};

type PlatformInfo = {
  isAndroid: boolean;
  tailscaleAddress: string | null;
  credentialStored: boolean;
  savedServer: string | null;
  savedUserId: string | null;
  notificationPermission: "granted" | "notRequested" | "denied" | "unavailable";
  fullScreenIntentPermission: boolean;
  pushFidReady: boolean;
  pushPaired: boolean;
  pushGatewayUrl: string | null;
  pushDeviceId: string | null;
  pushPairingError: string | null;
  pushSetupError: string | null;
  lastPushResult:
    | "registering"
    | "registered"
    | "completed"
    | "failed"
    | "ignored"
    | null;
  lastPushReceivedAt: number | null;
  lastPushRegisteredAt: number | null;
  lastPushDurationMillis: number | null;
  lastPushError: string | null;
};

type AppliedConnection = {
  server: string;
  userId: string;
};

type AudioRouteType =
  | "earpiece"
  | "speaker"
  | "wired"
  | "bluetooth"
  | "streaming"
  | "unknown";

type AudioRoute = {
  id: string;
  name: string;
  type: AudioRouteType;
};

type AudioRouteState = {
  status: "inactive" | "preparing" | "ready" | "error";
  current: AudioRoute | null;
  routes: AudioRoute[];
  bluetoothPermission:
    | "granted"
    | "notRequested"
    | "denied"
    | "unavailable";
  error: string | null;
};

type DesktopAlertMode = "normal" | "quiet";

type DesktopAlertSettings = {
  mode: DesktopAlertMode;
  volume: number;
  ringtoneFileName: string | null;
  outputDeviceId: string | null;
  outputDeviceName: string | null;
  previewing: boolean;
  playbackError: string | null;
  deviceFallback: boolean;
};

type DesktopAudioDevice = {
  id: string;
  name: string;
  isDefault: boolean;
};

const initialAudioRouteState: AudioRouteState = {
  status: "inactive",
  current: null,
  routes: [],
  bluetoothPermission: "unavailable",
  error: null,
};

const initialDesktopAlertSettings: DesktopAlertSettings = {
  mode: "normal",
  volume: 100,
  ringtoneFileName: null,
  outputDeviceId: null,
  outputDeviceName: null,
  previewing: false,
  playbackError: null,
  deviceFallback: false,
};

const initialPlatformInfo: PlatformInfo = {
  isAndroid: false,
  tailscaleAddress: null,
  credentialStored: false,
  savedServer: null,
  savedUserId: null,
  notificationPermission: "unavailable",
  fullScreenIntentPermission: false,
  pushFidReady: false,
  pushPaired: false,
  pushGatewayUrl: null,
  pushDeviceId: null,
  pushPairingError: null,
  pushSetupError: null,
  lastPushResult: null,
  lastPushReceivedAt: null,
  lastPushRegisteredAt: null,
  lastPushDurationMillis: null,
  lastPushError: null,
};

const keys = [
  ["1", ""],
  ["2", "ABC"],
  ["3", "DEF"],
  ["4", "GHI"],
  ["5", "JKL"],
  ["6", "MNO"],
  ["7", "PQRS"],
  ["8", "TUV"],
  ["9", "WXYZ"],
  ["*", ""],
  ["0", "+"],
  ["#", ""],
] as const;

const registrationLabels: Record<RegistrationState, string> = {
  stopped: "待受停止",
  platformReady: "接続中",
  pushReady: "待受中",
  registering: "接続中",
  registered: "待受中",
  retryWaiting: "接続中",
  failed: "接続失敗",
};

const pushResultLabels: Record<
  NonNullable<PlatformInfo["lastPushResult"]>,
  string
> = {
  registering: "SIP接続中",
  registered: "SIP登録成功",
  completed: "SIP登録成功（30秒で終了）",
  failed: "SIP接続失敗",
  ignored: "起動信号を無視",
};

function formatPushTime(timestamp: number | null) {
  if (timestamp === null) return "未実行";
  return new Intl.DateTimeFormat("ja-JP", {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(new Date(timestamp));
}

function formatCallDuration(totalSeconds: number) {
  const elapsed = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(elapsed / 3600);
  const minutes = Math.floor((elapsed % 3600) / 60);
  const seconds = elapsed % 60;
  const minuteText = minutes.toString().padStart(2, "0");
  const secondText = seconds.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${minuteText}:${secondText}`
    : `${minuteText}:${secondText}`;
}

function formatCallPeer(peer: string) {
  const trimmed = peer.trim();
  if (!trimmed) return "番号非通知";
  const sipMatch = trimmed.match(/sips?:([^@;>\s]+)/i);
  const identifier = sipMatch?.[1] ?? trimmed.split(/[@;>\s]/, 1)[0];
  if (!identifier || identifier.toLowerCase() === "anonymous") {
    return "番号非通知";
  }
  try {
    return decodeURIComponent(identifier);
  } catch {
    return identifier;
  }
}

function audioRouteTypeLabel(type: AudioRouteType) {
  switch (type) {
    case "earpiece":
      return "受話口";
    case "speaker":
      return "スピーカー";
    case "wired":
      return "有線";
    case "bluetooth":
      return "Bluetooth";
    case "streaming":
      return "外部機器";
    default:
      return "音声";
  }
}

function audioRouteIcon(type: AudioRouteType | undefined, size = 24) {
  switch (type) {
    case "earpiece":
      return <PhoneCall size={size} />;
    case "speaker":
      return <Speaker size={size} />;
    case "wired":
      return <Cable size={size} />;
    case "bluetooth":
      return <Bluetooth size={size} />;
    case "streaming":
      return <Cast size={size} />;
    default:
      return <Headphones size={size} />;
  }
}

function App() {
  const [tab, setTab] = useState<"phone" | "settings">("phone");
  const [phoneState, setPhoneState] = useState(initialPhoneState);
  const [platformInfo, setPlatformInfo] = useState(initialPlatformInfo);
  const [number, setNumber] = useState("");
  const [server, setServer] = useState("");
  const [userId, setUserId] = useState("");
  const [password, setPassword] = useState("");
  const [pushGatewayUrl, setPushGatewayUrl] = useState("");
  const [pairingCode, setPairingCode] = useState("");
  const [appliedConnection, setAppliedConnection] =
    useState<AppliedConnection | null>(null);
  const [error, setError] = useState("");
  const [pushMessage, setPushMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [audioRouteState, setAudioRouteState] = useState(
    initialAudioRouteState,
  );
  const [audioRouteOpen, setAudioRouteOpen] = useState(false);
  const [audioRouteBusy, setAudioRouteBusy] = useState(false);
  const [audioRouteError, setAudioRouteError] = useState("");
  const [desktopAlertSettings, setDesktopAlertSettings] = useState(
    initialDesktopAlertSettings,
  );
  const [desktopAudioDevices, setDesktopAudioDevices] = useState<
    DesktopAudioDevice[]
  >([]);
  const [desktopDeviceOpen, setDesktopDeviceOpen] = useState(false);
  const [desktopDeviceBusy, setDesktopDeviceBusy] = useState(false);
  const [desktopAlertBusy, setDesktopAlertBusy] = useState(false);
  const [desktopAlertError, setDesktopAlertError] = useState("");
  const audioRouteDialog = useRef<HTMLDialogElement>(null);
  const restoredAndroidSettings = useRef(false);
  const restoredPushSettings = useRef(false);
  const androidAudioRouteEligible =
    platformInfo.isAndroid &&
    phoneState.call !== null &&
    phoneState.call.phase !== "ringing" &&
    phoneState.call.phase !== "ending";

  useEffect(() => {
    const refresh = () => {
      invoke<PhoneState>("get_phone_state")
        .then(setPhoneState)
        .catch(() => undefined);
    };
    const platformRefresh = () => {
      invoke<PlatformInfo>("get_platform_info")
        .then((info) => {
          setPlatformInfo(info);
          if (info.isAndroid && !restoredAndroidSettings.current) {
            const restoredConnection = {
              server: info.savedServer ?? "",
              userId: info.savedUserId ?? "",
            };
            setServer(restoredConnection.server);
            setUserId(restoredConnection.userId);
            setAppliedConnection(restoredConnection);
            restoredAndroidSettings.current = true;
          }
          if (info.isAndroid && !restoredPushSettings.current) {
            setPushGatewayUrl(info.pushGatewayUrl ?? "");
            restoredPushSettings.current = true;
          }
        })
        .catch(() => undefined);
    };
    refresh();
    platformRefresh();
    const timer = window.setInterval(refresh, 500);
    const platformTimer = window.setInterval(platformRefresh, 2000);
    return () => {
      window.clearInterval(timer);
      window.clearInterval(platformTimer);
    };
  }, []);

  useEffect(() => {
    if (platformInfo.isAndroid) return;
    const refresh = () => {
      invoke<DesktopAlertSettings>("get_desktop_alert_settings")
        .then(setDesktopAlertSettings)
        .catch(() => undefined);
    };
    refresh();
    const timer = window.setInterval(refresh, 1000);
    return () => window.clearInterval(timer);
  }, [platformInfo.isAndroid]);

  useEffect(() => {
    if (
      platformInfo.isAndroid ||
      tab === "settings" ||
      !desktopAlertSettings.previewing
    )
      return;
    void invoke<DesktopAlertSettings>("stop_desktop_ringtone_preview")
      .then(setDesktopAlertSettings)
      .catch(() => undefined);
  }, [desktopAlertSettings.previewing, platformInfo.isAndroid, tab]);

  useEffect(() => {
    if (!desktopAlertSettings.previewing) return;
    const timer = window.setTimeout(() => {
      void invoke<DesktopAlertSettings>("stop_desktop_ringtone_preview")
        .then(setDesktopAlertSettings)
        .catch(() => undefined);
    }, 10_000);
    return () => window.clearTimeout(timer);
  }, [desktopAlertSettings.previewing]);

  useEffect(() => {
    if (!androidAudioRouteEligible) {
      setAudioRouteState(initialAudioRouteState);
      setAudioRouteOpen(false);
      setAudioRouteError("");
      return;
    }
    void refreshAudioRouteState();
    const timer = window.setInterval(() => {
      void refreshAudioRouteState();
    }, 500);
    return () => window.clearInterval(timer);
  }, [androidAudioRouteEligible]);

  useEffect(() => {
    const dialog = audioRouteDialog.current;
    if (!dialog) return;
    if (audioRouteOpen && !dialog.open) {
      dialog.showModal();
    } else if (!audioRouteOpen && dialog.open) {
      dialog.close();
    }
  }, [audioRouteOpen]);

  const statusTone = useMemo(() => {
    if (
      phoneState.registration === "registered" ||
      phoneState.registration === "pushReady"
    )
      return "ok";
    if (phoneState.registration === "failed") return "error";
    return "idle";
  }, [phoneState.registration]);

  const connectionChanged = useMemo(
    () =>
      appliedConnection !== null &&
      (server !== appliedConnection.server ||
        userId !== appliedConnection.userId ||
        password.length > 0),
    [appliedConnection, password, server, userId],
  );
  const currentAudioRouteLabel =
    audioRouteState.status === "preparing"
      ? "準備中"
      : audioRouteState.status === "error"
        ? "音声エラー"
        : audioRouteTypeLabel(audioRouteState.current?.type ?? "unknown");
  const dtmfAvailable =
    phoneState.call?.phase === "connecting" ||
    phoneState.call?.phase === "active";

  async function perform(action: () => Promise<PhoneState>) {
    setBusy(true);
    setError("");
    try {
      setPhoneState(await action());
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    } finally {
      setBusy(false);
    }
  }

  function pressKey(digit: string) {
    if (phoneState.call) {
      if (dtmfAvailable) {
        invoke("send_dtmf", { digit }).catch((reason) =>
          setError(String(reason)),
        );
      }
      return;
    }
    if (number.length < 32) setNumber((current) => `${current}${digit}`);
  }

  async function startWaiting(event: FormEvent) {
    event.preventDefault();
    const succeeded = await perform(() =>
      invoke("start_waiting", {
        request: {
          server,
          userId,
          password: password || null,
        },
      }),
    );
    if (succeeded) {
      setAppliedConnection({ server, userId });
      setPassword("");
      invoke<PlatformInfo>("get_platform_info")
        .then(setPlatformInfo)
        .catch(() => undefined);
    }
  }

  async function reconnect() {
    await perform(() => invoke("reconnect"));
  }

  async function runPushAction(
    action: () => Promise<void>,
    successMessage = "",
  ) {
    setBusy(true);
    setError("");
    setPushMessage("");
    try {
      await action();
      setPushMessage(successMessage);
      setPlatformInfo(await invoke<PlatformInfo>("get_platform_info"));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function refreshAudioRouteState() {
    try {
      setAudioRouteState(
        await invoke<AudioRouteState>("get_audio_route_state"),
      );
    } catch (reason) {
      setAudioRouteState((current) => ({
        ...current,
        status: "error",
        error: String(reason),
      }));
    }
  }

  async function openAudioRoutePanel() {
    setAudioRouteError("");
    setAudioRouteOpen(true);
    try {
      let state = await invoke<AudioRouteState>("get_audio_route_state");
      setAudioRouteState(state);
      if (state.bluetoothPermission === "notRequested") {
        await invoke("request_bluetooth_audio_permission");
        state = await invoke<AudioRouteState>("get_audio_route_state");
        setAudioRouteState(state);
      }
    } catch (reason) {
      setAudioRouteError(String(reason));
    }
  }

  async function selectAudioRoute(route: AudioRoute) {
    if (route.id === audioRouteState.current?.id) {
      setAudioRouteOpen(false);
      return;
    }
    setAudioRouteBusy(true);
    setAudioRouteError("");
    try {
      await invoke("set_audio_route", { routeId: route.id });
      await refreshAudioRouteState();
      setAudioRouteOpen(false);
    } catch (reason) {
      setAudioRouteError(String(reason));
    } finally {
      setAudioRouteBusy(false);
    }
  }

  async function openBluetoothAudioSettings() {
    setAudioRouteError("");
    try {
      await invoke("open_bluetooth_audio_settings");
    } catch (reason) {
      setAudioRouteError(String(reason));
    }
  }

  async function pairPushGateway() {
    await runPushAction(
      async () => {
        await invoke("pair_push_gateway", {
          request: {
            gatewayUrl: pushGatewayUrl,
            pairingCode,
          },
        });
        setPairingCode("");
      },
      "Push gatewayとペアリングしました",
    );
  }

  async function updateDesktopAlert(
    action: () => Promise<DesktopAlertSettings>,
  ) {
    setDesktopAlertBusy(true);
    setDesktopAlertError("");
    try {
      setDesktopAlertSettings(await action());
    } catch (reason) {
      setDesktopAlertError(String(reason));
    } finally {
      setDesktopAlertBusy(false);
    }
  }

  async function refreshDesktopAudioDevices() {
    setDesktopDeviceBusy(true);
    setDesktopAlertError("");
    try {
      setDesktopAudioDevices(
        await invoke<DesktopAudioDevice[]>(
          "list_desktop_ringtone_devices",
        ),
      );
    } catch (reason) {
      setDesktopAlertError(String(reason));
    } finally {
      setDesktopDeviceBusy(false);
    }
  }

  function toggleDesktopDevicePanel() {
    const opening = !desktopDeviceOpen;
    setDesktopDeviceOpen(opening);
    if (opening) void refreshDesktopAudioDevices();
  }

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-mark">
          <PhoneCall size={19} strokeWidth={2.2} />
        </div>
        <h1>Tsumuji Phone</h1>
        <div className={`status status-${statusTone}`}>
          <span aria-hidden="true" />
          {registrationLabels[phoneState.registration]}
        </div>
        {!platformInfo.isAndroid && (
          <div
            className={`desktop-alert-mode desktop-alert-mode-${desktopAlertSettings.mode}`}
            title={
              desktopAlertSettings.mode === "normal"
                ? "通常モード"
                : "着信を通知しないモード"
            }
          >
            {desktopAlertSettings.mode === "normal" ? (
              <Bell size={15} />
            ) : (
              <BellOff size={15} />
            )}
            <span>
              {desktopAlertSettings.mode === "normal"
                ? "通常"
                : "通知しない"}
            </span>
          </div>
        )}
      </header>

      <section className="content">
        {tab === "phone" ? (
          <div
            className={`dialer ${
              phoneState.call?.phase === "ringing"
                ? "incoming-dialer"
                : ""
            }`}
          >
            <div className="number-area">
              {phoneState.call ? (
                <div>
                  <span className="call-phase">
                    {phoneState.call.phase === "ringing" ? "着信" : "通話"}
                  </span>
                  <strong>{formatCallPeer(phoneState.call.peer)}</strong>
                  {phoneState.call.phase === "active" &&
                    phoneState.call.activeElapsedSeconds !== null && (
                      <span className="call-duration">
                        {formatCallDuration(
                          phoneState.call.activeElapsedSeconds,
                        )}
                      </span>
                    )}
                </div>
              ) : (
                <div className="number-input-wrap">
                  <input
                    aria-label="電話番号"
                    inputMode="tel"
                    value={number}
                    onChange={(event) =>
                      setNumber(
                        event.currentTarget.value
                          .replace(/[^0-9*#]/g, "")
                          .slice(0, 32),
                      )
                    }
                    placeholder="電話番号"
                  />
                  <button
                    className="icon-button"
                    type="button"
                    title="1文字削除"
                    disabled={!number}
                    onClick={() => setNumber((value) => value.slice(0, -1))}
                  >
                    <Delete size={21} />
                  </button>
                </div>
              )}
            </div>

            {phoneState.call?.phase !== "ringing" && (
              <div className="keypad" aria-label="ダイヤルキー">
                {keys.map(([digit, letters]) => (
                  <button
                    key={digit}
                    type="button"
                    className="key"
                    aria-label={
                      dtmfAvailable
                        ? `${digit}をDTMF送信`
                        : digit
                    }
                    disabled={phoneState.call !== null && !dtmfAvailable}
                    onClick={() => pressKey(digit)}
                  >
                    <span>{digit}</span>
                    <small>{letters || "\u00a0"}</small>
                  </button>
                ))}
              </div>
            )}

            <div
              className={`call-controls ${platformInfo.isAndroid ? "android-call-controls" : ""}`}
            >
              {phoneState.call?.phase === "ringing" ? (
                <>
                  <button
                    className="call-button answer"
                    type="button"
                    title="応答"
                    disabled={busy}
                    onClick={() => void perform(() => invoke("answer"))}
                  >
                    <Phone size={25} />
                  </button>
                  <button
                    className="call-button hangup"
                    type="button"
                    title="拒否"
                    disabled={busy}
                    onClick={() => void perform(() => invoke("reject"))}
                  >
                    <PhoneOff size={25} />
                  </button>
                </>
              ) : phoneState.call ? (
                platformInfo.isAndroid ? (
                  <>
                    <div className="call-control-item">
                      <button
                        className={`call-button secondary ${phoneState.muted ? "selected" : ""}`}
                        type="button"
                        title={
                          phoneState.muted ? "ミュート解除" : "ミュート"
                        }
                        disabled={busy || phoneState.call.phase !== "active"}
                        onClick={() =>
                          void perform(() =>
                            invoke("set_muted", {
                              muted: !phoneState.muted,
                            }),
                          )
                        }
                      >
                        {phoneState.muted ? (
                          <MicOff size={24} />
                        ) : (
                          <Mic size={24} />
                        )}
                      </button>
                      <span>{phoneState.muted ? "解除" : "ミュート"}</span>
                    </div>
                    <div className="call-control-item">
                      <button
                        className="call-button secondary"
                        type="button"
                        title={`音声経路: ${currentAudioRouteLabel}`}
                        disabled={
                          audioRouteBusy ||
                          audioRouteState.status !== "ready"
                        }
                        onClick={() => void openAudioRoutePanel()}
                      >
                        {audioRouteIcon(audioRouteState.current?.type)}
                      </button>
                      <span>{currentAudioRouteLabel}</span>
                    </div>
                    <div className="call-control-item">
                      <button
                        className="call-button hangup"
                        type="button"
                        title="通話終了"
                        disabled={busy}
                        onClick={() => void perform(() => invoke("hang_up"))}
                      >
                        <PhoneOff size={25} />
                      </button>
                      <span>切断</span>
                    </div>
                  </>
                ) : (
                  <>
                    <button
                      className={`call-button secondary ${phoneState.muted ? "selected" : ""}`}
                      type="button"
                      title={phoneState.muted ? "ミュート解除" : "ミュート"}
                      disabled={busy || phoneState.call.phase !== "active"}
                      onClick={() =>
                        void perform(() =>
                          invoke("set_muted", { muted: !phoneState.muted }),
                        )
                      }
                    >
                      {phoneState.muted ? (
                        <MicOff size={24} />
                      ) : (
                        <Mic size={24} />
                      )}
                    </button>
                    <button
                      className="call-button hangup"
                      type="button"
                      title="通話終了"
                      disabled={busy}
                      onClick={() => void perform(() => invoke("hang_up"))}
                    >
                      <PhoneOff size={25} />
                    </button>
                  </>
                )
              ) : (
                <button
                  className="call-button answer"
                  type="button"
                  title="発信"
                  disabled={
                    busy ||
                    !number ||
                    phoneState.registration !== "registered"
                  }
                  onClick={() =>
                    void perform(() => invoke("dial", { number }))
                  }
                >
                  <Phone size={25} />
                </button>
              )}
            </div>
            {platformInfo.isAndroid &&
              androidAudioRouteEligible &&
              audioRouteState.status === "error" && (
                <p className="audio-route-inline-error" role="alert">
                  {audioRouteState.error || "音声経路を利用できません"}
                </p>
              )}

            {(error || phoneState.lastError) && (
              <div className="error-message" role="alert">
                <ShieldAlert size={17} />
                <span>
                  {error || phoneState.lastError}
                  {platformInfo.isAndroid &&
                    phoneState.registration === "failed" && (
                      <button
                        className="inline-reconnect"
                        type="button"
                        disabled={busy || Boolean(phoneState.call)}
                        onClick={() => void reconnect()}
                      >
                        再接続
                      </button>
                    )}
                </span>
              </div>
            )}
          </div>
        ) : (
          <form className="settings" onSubmit={startWaiting}>
            <section>
              <h2>接続</h2>
              <label>
                SIPサーバー
                <input
                  value={server}
                  onChange={(event) => setServer(event.currentTarget.value)}
                  required
                />
              </label>
              <div className="platform-summary">
                <span>Tailscale IPv4</span>
                <strong>
                  {platformInfo.tailscaleAddress ??
                    "未接続／一意に検出できません"}
                </strong>
                <small>
                  {platformInfo.isAndroid ? "Android" : "Windows／Ubuntu"}
                  ではTailscaleのアドレスを自動的に使用します
                </small>
              </div>
              <label>
                ユーザID
                <input
                  value={userId}
                  minLength={1}
                  maxLength={32}
                  pattern={"[A-Za-z0-9._~\\-]+"}
                  onChange={(event) =>
                    setUserId(
                      event.currentTarget.value
                        .replace(/[^A-Za-z0-9._~-]/g, "")
                        .slice(0, 32),
                    )
                  }
                  required
                />
              </label>
              <label>
                パスワード
                <input
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  minLength={1}
                  maxLength={128}
                  onChange={(event) =>
                    setPassword(
                      event.currentTarget.value
                        .replace(/[^\x21-\x7e]/g, "")
                        .slice(0, 128),
                    )
                  }
                  placeholder="保存済みの場合は空欄"
                />
              </label>
              {platformInfo.isAndroid && platformInfo.credentialStored && (
                <p className="stored-credential">
                  パスワードはAndroid Keystoreで保護されています
                </p>
              )}
              {phoneState.registration === "stopped" ? (
                <button
                  className="primary-command"
                  type="submit"
                  disabled={busy || Boolean(phoneState.call)}
                >
                  <PhoneCall size={18} />
                  待受を開始
                </button>
              ) : (
                <div className="settings-actions">
                  {connectionChanged && (
                    <button
                      className="primary-command"
                      type="submit"
                      disabled={busy || Boolean(phoneState.call)}
                    >
                      <PhoneCall size={18} />
                      変更を適用
                    </button>
                  )}
                  <button
                    className="danger-command"
                    type="button"
                    disabled={busy || Boolean(phoneState.call)}
                    onClick={() => void perform(() => invoke("stop_waiting"))}
                  >
                    <PhoneOff size={18} />
                    待受を停止
                  </button>
                </div>
              )}
            </section>

            {platformInfo.isAndroid && (
              <section>
                <h2>Push待受</h2>
                <div className="push-summary">
                  <div>
                    <span>通知権限</span>
                    <strong>
                      {platformInfo.notificationPermission === "granted"
                        ? "許可済み"
                        : platformInfo.notificationPermission === "denied"
                          ? "未許可"
                          : "未設定"}
                    </strong>
                  </div>
                  <div>
                    <span>ロック画面の全画面着信</span>
                    <strong>
                      {platformInfo.fullScreenIntentPermission
                        ? "許可済み"
                        : "未許可"}
                    </strong>
                  </div>
                  <div>
                    <span>FCM</span>
                    <strong>
                      {platformInfo.pushFidReady ? "準備完了" : "準備中"}
                    </strong>
                    {platformInfo.pushSetupError && (
                      <small className="push-error">
                        {platformInfo.pushSetupError}
                      </small>
                    )}
                  </div>
                  <div>
                    <span>Push gateway</span>
                    <strong>
                      {platformInfo.pushPaired ? "ペアリング済み" : "未設定"}
                    </strong>
                    {platformInfo.pushDeviceId && (
                      <small>端末ID {platformInfo.pushDeviceId}</small>
                    )}
                    {platformInfo.pushPairingError && (
                      <small className="push-error">
                        {platformInfo.pushPairingError}
                      </small>
                    )}
                  </div>
                  <div>
                    <span>直近の起動信号</span>
                    <strong>
                      {platformInfo.lastPushResult
                        ? pushResultLabels[platformInfo.lastPushResult]
                        : "未実行"}
                    </strong>
                    {platformInfo.lastPushReceivedAt !== null && (
                      <small>
                        受信 {formatPushTime(platformInfo.lastPushReceivedAt)}
                        {platformInfo.lastPushDurationMillis !== null &&
                          `／SIP結果 ${(platformInfo.lastPushDurationMillis / 1000).toFixed(1)}秒`}
                      </small>
                    )}
                    {platformInfo.lastPushError && (
                      <small className="push-error">
                        {platformInfo.lastPushError}
                      </small>
                    )}
                  </div>
                </div>
                {platformInfo.notificationPermission !== "granted" && (
                  <button
                    className="secondary-command"
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void runPushAction(() =>
                        invoke("request_push_permission"),
                      )
                    }
                  >
                    <Bell size={18} />
                    通知を許可
                  </button>
                )}
                {platformInfo.notificationPermission === "denied" && (
                  <button
                    className="secondary-command"
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void runPushAction(() =>
                        invoke("open_push_notification_settings"),
                      )
                    }
                  >
                    <ExternalLink size={18} />
                    Androidの通知設定を開く
                  </button>
                )}
                {!platformInfo.fullScreenIntentPermission && (
                  <button
                    className="secondary-command"
                    type="button"
                    disabled={busy}
                    onClick={() =>
                      void runPushAction(() =>
                        invoke("open_full_screen_intent_settings"),
                      )
                    }
                  >
                    <ExternalLink size={18} />
                    全画面着信を許可
                  </button>
                )}
                <label>
                  Push gateway URL
                  <input
                    type="url"
                    inputMode="url"
                    autoCapitalize="none"
                    autoCorrect="off"
                    placeholder="https://server.tailnet.ts.net"
                    value={pushGatewayUrl}
                    onChange={(event) =>
                      setPushGatewayUrl(event.currentTarget.value)
                    }
                  />
                </label>
                <label>
                  ペアリングコード
                  <input
                    inputMode="text"
                    autoCapitalize="characters"
                    autoComplete="one-time-code"
                    maxLength={11}
                    placeholder="ABCDE-23456"
                    value={pairingCode}
                    onChange={(event) =>
                      setPairingCode(
                        event.currentTarget.value
                          .toUpperCase()
                          .replace(/[^2-9A-HJ-KM-NP-Z-]/g, "")
                          .slice(0, 11),
                      )
                    }
                  />
                </label>
                <button
                  className="secondary-command"
                  type="button"
                  disabled={
                    busy ||
                    !platformInfo.pushFidReady ||
                    !pushGatewayUrl ||
                    pairingCode.length < 10
                  }
                  onClick={() => void pairPushGateway()}
                >
                  Push gatewayとペアリング
                </button>
                {pushMessage && <p className="push-message">{pushMessage}</p>}
                <p className="platform-note">
                  通知を許可し、Push gatewayとペアリングした場合のみ、アプリ終了中の
                  起動信号を処理します。全画面着信を許可すると、ロック画面から
                  直接応答・拒否できます。サイレントモードやおやすみモードが
                  全画面表示を抑制している間は、通常の着信通知を表示します。
                </p>
              </section>
            )}

            {!platformInfo.isAndroid && (
              <section>
                <h2>音声デバイス</h2>
                <>
                  <label>
                    <span className="label-with-icon">
                      <Mic size={16} /> マイク
                    </span>
                    <select defaultValue="default">
                      <option value="default">システム既定</option>
                    </select>
                  </label>
                  <label>
                    <span className="label-with-icon">
                      <Headphones size={16} /> 通話音声
                    </span>
                    <select defaultValue="default">
                      <option value="default">システム既定</option>
                    </select>
                  </label>
                  <label>
                    <span className="label-with-icon">
                      <Volume2 size={16} /> 着信音
                    </span>
                    <div className="desktop-device-picker">
                      <button
                        className="desktop-device-trigger"
                        type="button"
                        aria-expanded={desktopDeviceOpen}
                        disabled={Boolean(phoneState.call)}
                        onClick={toggleDesktopDevicePanel}
                      >
                        {desktopAlertSettings.outputDeviceName ??
                          "システム既定"}
                      </button>
                      {desktopDeviceOpen && (
                        <div className="desktop-device-options">
                          {desktopDeviceBusy && (
                            <p>デバイスを検索中…</p>
                          )}
                          <button
                            type="button"
                            className={
                              desktopAlertSettings.outputDeviceId === null
                                ? "selected"
                                : ""
                            }
                            onClick={() => {
                              setDesktopDeviceOpen(false);
                              void updateDesktopAlert(() =>
                                invoke("set_desktop_ringtone_device", {
                                  deviceId: null,
                                }),
                              );
                            }}
                          >
                            <span>システム既定</span>
                            {desktopAlertSettings.outputDeviceId === null && (
                              <Check size={17} />
                            )}
                          </button>
                          {desktopAudioDevices.map((device) => (
                            <button
                              key={device.id}
                              type="button"
                              className={
                                desktopAlertSettings.outputDeviceId ===
                                device.id
                                  ? "selected"
                                  : ""
                              }
                              onClick={() => {
                                setDesktopDeviceOpen(false);
                                void updateDesktopAlert(() =>
                                  invoke("set_desktop_ringtone_device", {
                                    deviceId: device.id,
                                  }),
                                );
                              }}
                            >
                              <span>
                                {device.name}
                                {device.isDefault && (
                                  <small>現在のシステム既定</small>
                                )}
                              </span>
                              {desktopAlertSettings.outputDeviceId ===
                                device.id && <Check size={17} />}
                            </button>
                          ))}
                        </div>
                      )}
                    </div>
                  </label>
                </>
              </section>
            )}

            {!platformInfo.isAndroid && (
              <section className="ringtone-settings">
                <h2>着信設定</h2>
                <label>
                  通知モード
                  <select
                    value={desktopAlertSettings.mode}
                    disabled={desktopAlertBusy}
                    onChange={(event) =>
                      void updateDesktopAlert(() =>
                        invoke("set_desktop_alert_mode", {
                          mode: event.currentTarget.value,
                        }),
                      )
                    }
                  >
                    <option value="normal">通常モード</option>
                    <option value="quiet">
                      着信を通知しないモード
                    </option>
                  </select>
                </label>
                <p className="setting-help">
                  通常モードでは着信時に画面を表示して着信音を鳴らします。
                  通知しないモードでは、着信があっても画面や音で知らせません。
                </p>

                <div className="ringtone-file">
                  <span>着信音</span>
                  <strong>
                    {desktopAlertSettings.ringtoneFileName ??
                      "Tsumuji Phone 既定"}
                  </strong>
                  <div>
                    <button
                      className="secondary-command"
                      type="button"
                      disabled={
                        desktopAlertBusy || Boolean(phoneState.call)
                      }
                      onClick={() =>
                        void updateDesktopAlert(() =>
                          invoke("choose_desktop_ringtone"),
                        )
                      }
                    >
                      <FileAudio size={17} />
                      ファイルを選択
                    </button>
                    <button
                      className="secondary-command"
                      type="button"
                      disabled={
                        desktopAlertBusy ||
                        Boolean(phoneState.call) ||
                        desktopAlertSettings.ringtoneFileName === null
                      }
                      onClick={() =>
                        void updateDesktopAlert(() =>
                          invoke("reset_desktop_ringtone"),
                        )
                      }
                    >
                      <RotateCcw size={17} />
                      既定に戻す
                    </button>
                  </div>
                  <small>WAV、MP3、OGG Vorbis、FLACに対応</small>
                </div>

                <label>
                  <span className="ringtone-volume-label">
                    着信音量
                    <strong>{desktopAlertSettings.volume}</strong>
                  </span>
                  <div className="ringtone-volume">
                    <VolumeX size={18} />
                    <input
                      type="range"
                      min="0"
                      max="100"
                      value={desktopAlertSettings.volume}
                      disabled={desktopAlertBusy}
                      onChange={(event) => {
                        const volume = Number(event.currentTarget.value);
                        setDesktopAlertSettings((current) => ({
                          ...current,
                          volume,
                        }));
                        void invoke<DesktopAlertSettings>(
                          "set_desktop_ringtone_volume",
                          { volume },
                        )
                          .then(setDesktopAlertSettings)
                          .catch((reason) =>
                            setDesktopAlertError(String(reason)),
                          );
                      }}
                    />
                    <Volume2 size={18} />
                  </div>
                </label>

                <button
                  className="secondary-command ringtone-preview"
                  type="button"
                  disabled={
                    desktopAlertBusy || Boolean(phoneState.call)
                  }
                  onClick={() =>
                    void updateDesktopAlert(() =>
                      invoke(
                        desktopAlertSettings.previewing
                          ? "stop_desktop_ringtone_preview"
                          : "start_desktop_ringtone_preview",
                      ),
                    )
                  }
                >
                  {desktopAlertSettings.previewing ? (
                    <>
                      <VolumeX size={18} />
                      停止
                    </>
                  ) : (
                    <>
                      <Play size={18} />
                      試聴
                    </>
                  )}
                </button>

                {(desktopAlertError ||
                  desktopAlertSettings.playbackError) && (
                  <p className="ringtone-error" role="alert">
                    {desktopAlertError ||
                      desktopAlertSettings.playbackError}
                  </p>
                )}
              </section>
            )}

            {(error || phoneState.lastError) && (
              <div className="error-message" role="alert">
                <ShieldAlert size={17} />
                <span>{error || phoneState.lastError}</span>
              </div>
            )}
          </form>
        )}
      </section>

      {platformInfo.isAndroid && (
        <dialog
          ref={audioRouteDialog}
          className="audio-route-dialog"
          aria-labelledby="audio-route-title"
          onCancel={(event) => {
            event.preventDefault();
            setAudioRouteOpen(false);
          }}
          onClose={() => setAudioRouteOpen(false)}
          onClick={(event) => {
            if (event.target === event.currentTarget) {
              setAudioRouteOpen(false);
            }
          }}
        >
          <div className="audio-route-sheet">
            <header>
              <div>
                <h2 id="audio-route-title">通話音声</h2>
                <p>使用する音声経路を選択してください</p>
              </div>
              <button
                className="audio-route-close"
                type="button"
                title="閉じる"
                onClick={() => setAudioRouteOpen(false)}
              >
                <X size={22} />
              </button>
            </header>

            <div className="audio-route-options">
              {audioRouteState.routes.map((route) => {
                const selected = route.id === audioRouteState.current?.id;
                return (
                  <button
                    key={route.id}
                    type="button"
                    className={selected ? "selected" : ""}
                    disabled={audioRouteBusy}
                    onClick={() => void selectAudioRoute(route)}
                  >
                    <span className="audio-route-icon">
                      {audioRouteIcon(route.type, 21)}
                    </span>
                    <span>
                      <strong>{route.name}</strong>
                      <small>{audioRouteTypeLabel(route.type)}</small>
                    </span>
                    {selected && <Check size={21} aria-label="選択中" />}
                  </button>
                );
              })}
            </div>

            {audioRouteState.bluetoothPermission === "denied" && (
              <div className="audio-route-permission">
                <p>
                  Bluetooth機器を使うには「付近のデバイス」を許可してください。
                </p>
                <button
                  type="button"
                  disabled={audioRouteBusy}
                  onClick={() => void openBluetoothAudioSettings()}
                >
                  <ExternalLink size={17} />
                  Android設定を開く
                </button>
              </div>
            )}

            {(audioRouteError || audioRouteState.error) && (
              <p className="audio-route-error" role="alert">
                {audioRouteError || audioRouteState.error}
              </p>
            )}
          </div>
        </dialog>
      )}

      <nav className="bottom-nav" aria-label="メイン">
        <button
          className={tab === "phone" ? "active" : ""}
          type="button"
          onClick={() => setTab("phone")}
        >
          <Phone size={20} />
          <span>電話</span>
        </button>
        <button
          className={tab === "settings" ? "active" : ""}
          type="button"
          onClick={() => setTab("settings")}
        >
          <Settings size={20} />
          <span>設定</span>
        </button>
      </nav>
    </main>
  );
}

export default App;
