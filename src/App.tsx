import { FormEvent, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Delete,
  Headphones,
  Mic,
  MicOff,
  Phone,
  PhoneCall,
  PhoneOff,
  Settings,
  ShieldAlert,
  Volume2,
} from "lucide-react";
import "./App.css";

type RegistrationState =
  | "stopped"
  | "registering"
  | "registered"
  | "retryWaiting"
  | "failed";

type Call = {
  id: number;
  peer: string;
  direction: "incoming" | "outgoing";
  phase: "ringing" | "dialing" | "connecting" | "active" | "ending";
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
  registering: "登録中",
  registered: "待受中",
  retryWaiting: "再接続待ち",
  failed: "接続失敗",
};

function App() {
  const [tab, setTab] = useState<"phone" | "settings">("phone");
  const [phoneState, setPhoneState] = useState(initialPhoneState);
  const [number, setNumber] = useState("");
  const [server, setServer] = useState("");
  const [localAddress, setLocalAddress] = useState("");
  const [userId, setUserId] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const refresh = () => {
      invoke<PhoneState>("get_phone_state")
        .then(setPhoneState)
        .catch(() => undefined);
    };
    refresh();
    const timer = window.setInterval(refresh, 500);
    return () => window.clearInterval(timer);
  }, []);

  const statusTone = useMemo(() => {
    if (phoneState.registration === "registered") return "ok";
    if (phoneState.registration === "failed") return "error";
    return "idle";
  }, [phoneState.registration]);

  async function perform(action: () => Promise<PhoneState>) {
    setBusy(true);
    setError("");
    try {
      setPhoneState(await action());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function pressKey(digit: string) {
    if (phoneState.call?.phase === "active") {
      invoke("send_dtmf", { digit }).catch((reason) => setError(String(reason)));
      return;
    }
    if (number.length < 32) setNumber((current) => `${current}${digit}`);
  }

  function startWaiting(event: FormEvent) {
    event.preventDefault();
    void perform(() =>
      invoke("start_waiting", {
        request: {
          server,
          localAddress,
          userId,
          password: password || null,
        },
      }),
    ).then(() => setPassword(""));
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
      </header>

      <section className="content">
        {tab === "phone" ? (
          <div className="dialer">
            <div className="number-area">
              {phoneState.call ? (
                <div>
                  <span className="call-phase">
                    {phoneState.call.phase === "ringing" ? "着信" : "通話"}
                  </span>
                  <strong>{phoneState.call.peer || "番号非通知"}</strong>
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

            <div className="keypad" aria-label="ダイヤルキー">
              {keys.map(([digit, letters]) => (
                <button
                  key={digit}
                  type="button"
                  className="key"
                  onClick={() => pressKey(digit)}
                >
                  <span>{digit}</span>
                  <small>{letters || "\u00a0"}</small>
                </button>
              ))}
            </div>

            <div className="call-controls">
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
                    {phoneState.muted ? <MicOff size={24} /> : <Mic size={24} />}
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

            {(error || phoneState.lastError) && (
              <div className="error-message" role="alert">
                <ShieldAlert size={17} />
                <span>{error || phoneState.lastError}</span>
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
              <label>
                この端末のIPアドレス
                <input
                  value={localAddress}
                  onChange={(event) => setLocalAddress(event.currentTarget.value)}
                  required
                />
              </label>
              <label>
                ユーザID
                <input
                  value={userId}
                  minLength={1}
                  maxLength={32}
                  pattern="[A-Za-z0-9._~-]+"
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
              {phoneState.registration === "stopped" ? (
                <button className="primary-command" type="submit" disabled={busy}>
                  <PhoneCall size={18} />
                  待受を開始
                </button>
              ) : (
                <button
                  className="danger-command"
                  type="button"
                  disabled={busy}
                  onClick={() => void perform(() => invoke("stop_waiting"))}
                >
                  <PhoneOff size={18} />
                  待受を停止
                </button>
              )}
            </section>

            <section>
              <h2>音声デバイス</h2>
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
                <select defaultValue="default">
                  <option value="default">システム既定</option>
                </select>
              </label>
            </section>

            {(error || phoneState.lastError) && (
              <div className="error-message" role="alert">
                <ShieldAlert size={17} />
                <span>{error || phoneState.lastError}</span>
              </div>
            )}
          </form>
        )}
      </section>

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
