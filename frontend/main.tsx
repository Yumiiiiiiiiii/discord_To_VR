import React, { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowRight,
  Activity,
  Download,
  Bell,
  BellRing,
  Check,
  CheckCheck,
  ChevronRight,
  CircleHelp,
  ExternalLink,
  FolderOpen,
  Gamepad2,
  Home,
  Info,
  LockKeyhole,
  ListFilter,
  Plus,
  MessageCircle,
  Monitor,
  Power,
  Radio,
  Settings2,
  Shield,
  ShieldCheck,
  Sparkles,
  Volume2,
  X,
} from "lucide-react";
import {
  api,
  channelIdFromInput,
  desktop,
  errorMessage,
  type Settings,
  type SettingsInput,
  type Status,
  type ReceiveMode,
  type ThemePreference,
} from "./bridge";
import "./styles.css";
import { Onboarding } from "./onboarding";
import { DiagnosticsPanel } from "./diagnostics";
import { UpdatesPanel, UpdateNotice, useUpdateStatus } from "./updates";

type Page = "home" | "settings" | "tutorial" | "debug" | "updates";
type Tab = "display" | "sound" | "discord" | "whitelist";
type Form = Settings & {
  clientSecret: string;
  botToken: string;
  clearBotToken: boolean;
};
const emptyStatus: Status = {
  theme: "system",
  discord: 0,
  overlay: false,
  privacy: false,
  configured: false,
  paused: false,
  tray: false,
  problem: null,
  preview: !desktop,
  startSettings: false,
  receiveMode: "normal",
  whitelistOnly: false,
  whitelistCount: 0,
  whitelistStatus: 0,
  botStatus: "disabled",
  botConfigured: false,
  onboardingPending: false,
  debugEnabled: false,
};
const tabs: { id: Tab; label: string; icon: typeof Monitor }[] = [
  { id: "display", label: "通知の表示", icon: Monitor },
  { id: "sound", label: "通知音・動作", icon: Volume2 },
  { id: "discord", label: "Discord接続", icon: MessageCircle },
  { id: "whitelist", label: "許可リスト", icon: ListFilter },
];
const receiveChoices: { mode: ReceiveMode; label: string; help: string }[] = [
  {
    mode: "normal",
    label: "Discord通知に従う",
    help: "通常どおり通知イベントを転送",
  },
  {
    mode: "whitelist",
    label: "許可リストだけ",
    help: "登録したチャンネル・DMだけ受信",
  },
  {
    mode: "auto",
    label: "Botで自動判別",
    help: "取り込み中は許可リストだけ受信",
  },
];
const botLabels = {
  disabled: "監視停止中",
  connecting: "Bot接続中",
  unknown: "判定待ち",
  dnd: "取り込み中",
  online: "オンライン",
  idle: "退席中",
  offline: "オフライン・判定保留",
  error: "Token・Presence Intentを確認",
  missing_guild: "指定サーバーにBotがいません",
  missing_user: "指定サーバーに自分がいません",
  account_mismatch: "RPCのアカウントとユーザーIDが違います",
};

function App() {
  const updateStatus=useUpdateStatus();
  const [page, setPage] = useState<Page>("home");
  const [tab, setTab] = useState<Tab>("display");
  const [status, setStatus] = useState<Status>(emptyStatus);
  const [form, setForm] = useState<Form | null>(null);
  const theme = form?.theme ?? status.theme;
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    if (desktop) void api("preview_theme", { theme }).catch(error =>
      setToast({ message: errorMessage(error), error: true }));
  }, [theme]);
  const [tutorialSettings, setTutorialSettings] = useState<Settings | null>(null);
  const [tutorialResume, setTutorialResume] = useState(false);
  const [busy, setBusy] = useState(false);
  const [channelInput, setChannelInput] = useState("");
  const [channelError, setChannelError] = useState<string | null>(null);
  const [toast, setToast] = useState<{
    message: string;
    error: boolean;
  } | null>(null);
  const [formError, setFormError] = useState<{
    field?: string;
    message: string;
  } | null>(null);
  const formRef = useRef<HTMLFormElement>(null);
  const ready = status.discord === 2 && (status.debugEnabled || status.overlay) && !status.paused;
  const refresh = useCallback(async () => {
    try {
      setStatus(await api<Status>("get_status"));
    } catch (e) {
      setToast({ message: errorMessage(e), error: true });
    }
  }, []);
  const notify = (message: string, error = false) =>
    setToast({ message, error });
  const settings = useCallback(async (section: Tab = "display") => {
    setBusy(true);
    setFormError(null);
    setChannelInput("");
    setChannelError(null);
    try {
      const values = await api<Settings>("get_settings");
      setTutorialSettings(null);
      setStatus((previous) => ({ ...previous, paused: true, overlay: false }));
      setForm({
        ...values,
        clientSecret: "",
        botToken: "",
        clearBotToken: false,
      });
      setTab(section);
      setPage("settings");
    } catch (e) {
      setToast({ message: errorMessage(e), error: true });
    } finally {
      setBusy(false);
    }
  }, []);
  const tutorial = useCallback(async (resumeConnection = false) => {
    setBusy(true);
    try {
      const values = await api<Settings>("get_settings");
      if (resumeConnection) await api("cancel_settings");
      setForm(null);
      setTutorialSettings(values);
      setTutorialResume(resumeConnection);
      setStatus(previous => ({ ...previous, paused: !resumeConnection, overlay: false }));
      setPage("tutorial");
    } catch (e) { setToast({ message: errorMessage(e), error: true }); }
    finally { setBusy(false); }
  }, []);
  async function openDebug() {
    setBusy(true);
    try {
      if (page === "settings" || page === "tutorial") await api("cancel_settings");
      setForm(null); setTutorialSettings(null); setPage("debug");
    } catch(e) {notify(errorMessage(e), true);}
    finally {setBusy(false);await refresh();}
  }
  async function openUpdates() {
    setBusy(true);
    try {
      if(page==="settings"||page==="tutorial")await api("cancel_settings");
      setForm(null);setTutorialSettings(null);setPage("updates");
    }catch(e){notify(errorMessage(e),true);}
    finally{setBusy(false);await refresh();}
  }
  useEffect(() => {
    let active = true;
    api<Status>("get_status")
      .then((value) => {
        if (active) {
          setStatus(value);
          if (value.startSettings)
            void settings(value.configured ? "display" : "discord");
          else if (value.debugEnabled) setPage("debug");
          else if (value.onboardingPending) void tutorial(value.configured);
          else if (!value.configured) void settings("discord");
        }
      })
      .catch(
        (e) => active && setToast({ message: errorMessage(e), error: true }),
      );
    const timer = setInterval(() => {
      if (active) void refresh();
    }, 1000);
    const subscriptions = desktop
      ? [
          listen("navigate-settings", () => void settings()),
          listen("navigate-home", () => {
            setForm(null);
            setTutorialSettings(null);
            setPage("home");
            void refresh();
          }),
          listen<string>("operation-error", (event) =>
            notify(event.payload, true),
          ),
          listen<{ Ok?: string; Err?: string }>("operation-result", (event) =>
            notify(
              event.payload.Ok ?? event.payload.Err ?? "",
              Boolean(event.payload.Err),
            ),
          ),
        ]
      : [];
    return () => {
      active = false;
      clearInterval(timer);
      subscriptions.forEach((p) => void p.then((unlisten) => unlisten()));
    };
  }, [refresh, settings, tutorial]);
  useEffect(() => {
    const hide = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || busy || !desktop) return;
      event.preventDefault();
      void api("hide_to_tray").catch((error) =>
        setToast({ message: errorMessage(error), error: true }),
      );
    };
    window.addEventListener("keydown", hide);
    return () => window.removeEventListener("keydown", hide);
  }, [busy]);
  useEffect(() => {
    if (!toast || toast.error) return;
    const timer = setTimeout(() => setToast(null), 5000);
    return () => clearTimeout(timer);
  }, [toast]);
  async function mode(privacy: boolean) {
    setBusy(true);
    setStatus((previous) => ({ ...previous, privacy }));
    try {
      await api("set_privacy", { enabled: privacy });
      notify("表示モードを保存しました。次回もこのモードで起動します。");
    } catch (e) {
      notify(errorMessage(e), true);
    } finally {
      setBusy(false);
      await refresh();
    }
  }
  async function receiveMode(mode: ReceiveMode) {
    setBusy(true);
    try {
      await api("set_receive_mode", { mode });
      notify("受信モードを切り替えました。");
    } catch (error) {
      notify(errorMessage(error), true);
    } finally {
      setBusy(false);
      await refresh();
    }
  }
  async function cancel() {
    setBusy(true);
    try {
      await api("cancel_settings");
      setForm(null);
      setTutorialSettings(null);
      setPage("home");
      setFormError(null);
    } catch (e) {
      notify(errorMessage(e), true);
    } finally {
      setBusy(false);
      await refresh();
    }
  }
  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!form) return;
    setBusy(true);
    setFormError(null);
    const {
      secretSet: _secretSet,
      botTokenSet: _botTokenSet,
      clientSecret,
      botToken,
      ...values
    } = form;
    const input: SettingsInput = {
      ...values,
      clientSecret: clientSecret.trim() || null,
      botToken: botToken.trim() || null,
    };
    try {
      const saved = await api<Settings>("save_settings", { input });
      setStatus(previous => ({ ...previous, theme: saved.theme }));
      setForm(null);
      setPage("home");
      notify("設定を保存しました。接続を再開します。");
    } catch (error) {
      const field =
        error && typeof error === "object" && "field" in error
          ? String(error.field ?? "")
          : "";
      setFormError({ field, message: errorMessage(error) });
      if (["clientId", "clientSecret", "redirectUri"].includes(field))
        setTab("discord");
      else if (
        ["whitelistChannelIds", "botToken", "botGuildId", "botUserId"].includes(
          field,
        )
      )
        setTab("whitelist");
      else if (["volumePercent", "notificationSound"].includes(field))
        setTab("sound");
      else if (field) setTab("display");
      setTimeout(
        () =>
          formRef.current
            ?.querySelector<HTMLInputElement>(`[name="${field}"]`)
            ?.focus(),
        0,
      );
    } finally {
      setBusy(false);
      await refresh();
    }
  }
  async function saveTutorial(input: SettingsInput) {
    setBusy(true);
    try { return await api<Settings>("save_settings", { input }); }
    finally { setBusy(false); await refresh(); }
  }
  async function finishTutorial(extras = false) {
    setBusy(true);
    try {
      await api("complete_onboarding");
      await api("cancel_settings");
      setTutorialSettings(null);
      setPage("home");
      if (extras) await settings("whitelist");
    } finally { setBusy(false); await refresh(); }
  }
  function change<K extends keyof Form>(key: K, value: Form[K]) {
    setForm((previous) =>
      previous ? { ...previous, [key]: value } : previous,
    );
  }
  async function action(command: string, success?: string) {
    setBusy(true);
    try {
      await api(command);
      if (success) notify(success);
    } catch (e) {
      notify(errorMessage(e), true);
    } finally {
      setBusy(false);
    }
  }
  const fieldError = (name: string) => formError?.field === name;
  const numberField = (
    name: "notificationTimeout" | "maxTitleLength" | "maxContentLength",
    label: string,
    help: string,
    min: number,
    max: number,
    unit: string,
  ) =>
    form && (
      <div className="setting-row">
        <div>
          <label htmlFor={name}>{label}</label>
          <p>{help}</p>
        </div>
        <div className="number-field">
          <input
            id={name}
            name={name}
            type="number"
            min={min}
            max={max}
            step={name === "notificationTimeout" ? "0.5" : "1"}
            required
            value={form[name]}
            aria-invalid={fieldError(name)}
            onChange={(e) => change(name, Number(e.target.value))}
          />
          <span>{unit}</span>
        </div>
      </div>
    );

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">
            <Gamepad2 size={25} strokeWidth={1.8} />
          </span>
          <div>
            <strong>
              Discord <span>→</span> VR
            </strong>
            <small>通知をつなぐ、小さな窓。</small>
          </div>
        </div>
        <div className="nav-label">ワークスペース</div>
        <nav aria-label="メイン">
          <button
            className={page === "home" ? "active" : ""}
            onClick={() => (page !== "home" ? void cancel() : undefined)}
            disabled={busy}
          >
            <Home size={18} />
            ホーム{page === "home" && <span className="nav-dot" />}
          </button>
          {status.debugEnabled && <button className={page==="debug"?"active":""} disabled={busy||page==="debug"} onClick={()=>void openDebug()}><Activity size={18}/>受信診断{page==="debug"&&<span className="nav-dot"/>}</button>}
          <button className={page === "tutorial" ? "active" : ""} disabled={busy || page === "tutorial"} onClick={() => void tutorial()}>
            <CircleHelp size={18} />
            使い方ガイド{page === "tutorial" && <span className="nav-dot" />}
          </button>
          <button
            className={page === "settings" ? "active" : ""}
            onClick={() =>
              void settings(status.configured ? "display" : "discord")
            }
            disabled={busy || page === "settings"}
          >
            <Settings2 size={18} />
            設定{page === "settings" && <span className="nav-dot" />}
          </button>
          <button className={page==="updates"?"active":""} disabled={busy||page==="updates"} onClick={()=>void openUpdates()}><Download size={18}/>アプリの更新</button>
        </nav>
        <div className="sidebar-bottom">
          <div className="mode-indicator">
            <ShieldCheck size={19} />
            <div>
              <span>現在の表示モード</span>
              <strong>{status.privacy ? "配信用表示" : "通常表示"}</strong>
            </div>
          </div>
          <div className="local-note">
            <LockKeyhole size={13} />
            認証情報はこの PC で保護
          </div>
          <button
            className="quit-button"
            onClick={() => void action("exit_app")}
            disabled={busy}
          >
            <Power size={15} />
            アプリを終了
          </button>
          <small className="version">
            Discord to VR <span>v{updateStatus?.currentVersion??"0.2.1"}</span>
          </small>
        </div>
      </aside>
      <div className="workspace">
        <header className="topbar">
          <div className="breadcrumb">
            Discord to VR <ChevronRight size={13} />
            <span>{page === "home" ? "ホーム" : page === "tutorial" ? "使い方ガイド" : page === "debug" ? "受信診断" : page === "updates" ? "アプリの更新" : "設定"}</span>
          </div>
          <span className={`connection-pill ${ready ? "connected" : ""}`}>
            <i />
            {status.paused
              ? "設定中 · 転送を一時停止"
              : ready
                ? status.debugEnabled ? "Discord 受信テスト中" : "接続済み"
                : "接続待ち"}
          </span>
        </header>
        <main className="main-content">
          {page!=="updates"&&<UpdateNotice status={updateStatus} onOpen={()=>void openUpdates()}/>}
          {status.preview && (
            <div className="preview-banner">
              <Info size={15} />
              画面プレビュー · Discord
              への接続、通知の送信、設定ファイルの保存は行いません。
            </div>
          )}
          {status.problem && (
            <div className="error-banner" role="alert">
              <CircleHelp size={18} />
              {status.problem}
            </div>
          )}
          {page === "updates" ? <UpdatesPanel status={updateStatus}/> : page === "debug" && status.debugEnabled ? (
            <DiagnosticsPanel status={status} onSetup={()=>void (status.configured?settings("discord"):tutorial())} onWhitelist={()=>void settings("whitelist")}/>
          ) : page === "tutorial" && tutorialSettings ? (
            <Onboarding initial={tutorialSettings} status={status} busy={busy} resumeConnection={tutorialResume} onSave={saveTutorial} onEdit={() => api<Settings>("get_settings")} onFinish={finishTutorial} onLater={cancel} />
          ) : page === "home" ? (
            <>
              <div className="page-heading">
                <div>
                  <div className="eyebrow">YOUR NOTIFICATIONS, IN VR</div>
                  <h1>通知を、VR の中へ。</h1>
                  <p>いつものメッセージを、もっと自然に受け取ろう。</p>
                </div>
                <span className="heading-icon">
                  <BellRing size={27} strokeWidth={1.5} />
                </span>
              </div>
              {(!status.configured || status.onboardingPending) && (
                <div className="setup-prompt">
                  <div>
                    <strong>{status.configured ? "初回の案内を続けましょう" : "最初の接続設定をしましょう"}</strong>
                    <p>
                      {status.onboardingPending ? "準備から接続確認まで、順番にご案内します。" : "自分の Discord アプリの Client ID を使って接続します。"}
                    </p>
                  </div>
                  <button
                    className="primary"
                    onClick={() => void (status.onboardingPending ? tutorial(status.configured) : settings("discord"))}
                  >
                    {status.onboardingPending ? "案内を再開" : "接続を設定"}
                    <ArrowRight size={15} />
                  </button>
                </div>
              )}
              <div className="connection-grid">
                <article className="connection-card">
                  <div className="connection-card-top">
                    <span className="app-icon discord-icon">
                      <MessageCircle size={23} />
                    </span>
                    <span
                      className={`status-label ${status.discord === 2 && !status.paused ? "good" : ""}`}
                    >
                      <i />
                      {status.paused
                        ? "一時停止"
                        : ([
                            "起動待ち",
                            "認証中",
                            "接続済み",
                            "再接続中",
                            "確認が必要",
                          ][status.discord] ?? "接続待ち")}
                    </span>
                  </div>
                  <h2>Discord</h2>
                  <p>
                    {status.paused
                      ? "設定を閉じると受信を再開します。"
                      : status.discord === 2
                        ? "デスクトップアプリの通知を受信できます。"
                        : status.discord === 4
                          ? "認証情報または承認状態を確認してください。"
                          : status.discord === 1
                            ? "Discord に表示された承認画面を確認してください。"
                            : "Discord デスクトップアプリを起動してください。"}
                  </p>
                  <div className="connection-meta">
                    <span>
                      <Monitor size={12} />
                      デスクトップアプリ
                    </span>
                    {status.discord === 4 && (
                      <button onClick={() => void settings("discord")}>
                        設定を確認
                        <ChevronRight size={12} />
                      </button>
                    )}
                  </div>
                </article>
                <article className="connection-card">
                  <div className="connection-card-top">
                    <span className="app-icon overlay-icon">
                      <Gamepad2 size={24} />
                    </span>
                    <span
                      className={`status-label ${status.overlay && !status.paused ? "good" : ""}`}
                    >
                      <i />
                      {status.debugEnabled
                        ? "受信テストでは停止"
                        : status.paused
                        ? "一時停止"
                        : status.overlay
                          ? "接続済み"
                          : "接続待ち"}
                    </span>
                  </div>
                  <h2>XSOverlay</h2>
                  <p>
                    {status.debugEnabled
                      ? "-debug 起動では XSOverlay を使わず Discord の受信だけを確認します。"
                      : status.paused
                      ? "設定を閉じると送信を再開します。"
                      : status.overlay
                        ? "VR 内へ通知を届ける準備ができています。"
                        : "XSOverlay を起動し、通知用 WebSocket を有効にしてください。"}
                  </p>
                  <div className="connection-meta">
                    <span>
                      <Radio size={12} />
                      VR の通知表示
                    </span>
                    <span>
                      {status.debugEnabled ? "VR へは送信しません" : status.overlay ? "準備完了" : "接続を待っています"}
                    </span>
                  </div>
                </article>
              </div>
              <section className="settings-card receive-card">
                <div className="section-heading">
                  <div>
                    <h2>通知の受信</h2>
                    <p>取り込み中も、必要な会話だけを受け取れます。</p>
                  </div>
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() => void settings("whitelist")}
                  >
                    <ListFilter size={15} />
                    許可リストを設定
                  </button>
                </div>
                <div className="receive-options">
                  {receiveChoices.map((choice) => (
                    <button
                      key={choice.mode}
                      className={`receive-option ${status.receiveMode === choice.mode ? "selected" : ""}`}
                      aria-pressed={status.receiveMode === choice.mode}
                      disabled={
                        busy ||
                        !status.configured ||
                        (choice.mode === "auto" && !status.botConfigured)
                      }
                      onClick={() => void receiveMode(choice.mode)}
                    >
                      <span>
                        <strong>{choice.label}</strong>
                        <small>{choice.help}</small>
                      </span>
                      <span className="radio-check">
                        {status.receiveMode === choice.mode && (
                          <Check size={12} />
                        )}
                      </span>
                    </button>
                  ))}
                </div>
                <div className="receive-summary">
                  <ShieldCheck size={16} />
                  <span>
                    {status.whitelistOnly
                      ? `受信対象: 許可リスト ${status.whitelistCount} 件だけ`
                      : "受信対象: Discord が通知した会話"}
                    {status.receiveMode === "auto" &&
                      ` · ${botLabels[status.botStatus]}`}
                  </span>
                </div>
                {status.receiveMode === "auto" &&
                  status.whitelistOnly &&
                  status.botStatus !== "dnd" && (
                    <p className="receive-help">
                      判定できるまで許可リストだけを流します。Bot
                      と自分が同じサーバーにいること、Presence Intent
                      を確認してください。
                    </p>
                  )}
                {status.whitelistCount > 0 && status.whitelistStatus !== 2 && (
                  <p className="receive-help">
                    {status.whitelistStatus === 3
                      ? "許可リストの一部を購読できませんでした。ID・閲覧権限・messages.read の承認を確認してください。"
                      : "許可リストの購読待ちです。Discord の承認画面でメッセージ読み取りを許可してください。"}
                  </p>
                )}
                {status.preview && status.receiveMode === "auto" && (
                  <div className="preview-presence">
                    <span>判定の体験（ダミー）</span>
                    {[
                      { status: "dnd", label: "取り込み中" },
                      { status: "online", label: "オンライン" },
                      { status: "offline", label: "オフライン" },
                    ].map((sample) => (
                      <button
                        key={sample.status}
                        className="secondary"
                        onClick={async () => {
                          await api("preview_set_presence", {
                            status: sample.status,
                          });
                          await refresh();
                        }}
                      >
                        {sample.label}
                      </button>
                    ))}
                  </div>
                )}
              </section>
              <section className="display-card">
                <div className="section-heading">
                  <div>
                    <h2>通知の表示</h2>
                    <p>選んだモードは、次の起動でも引き継がれます。</p>
                  </div>
                  <span className="subtle-tag">
                    <CheckCheck size={13} />
                    自動保存
                  </span>
                </div>
                <div className="display-body">
                  <div className="mode-options">
                    <button
                      className={`mode-option ${!status.privacy ? "selected" : ""}`}
                      aria-pressed={!status.privacy}
                      disabled={busy || !status.configured}
                      onClick={() => void mode(false)}
                    >
                      <span className="mode-option-icon">
                        <MessageCircle size={22} />
                      </span>
                      <span>
                        <strong>通常表示</strong>
                        <small>名前・本文・アイコンを表示</small>
                      </span>
                      <span className="radio-check">
                        {!status.privacy && <Check size={12} strokeWidth={3} />}
                      </span>
                    </button>
                    <button
                      className={`mode-option ${status.privacy ? "selected" : ""}`}
                      aria-pressed={status.privacy}
                      disabled={busy || !status.configured}
                      onClick={() => void mode(true)}
                    >
                      <span className="mode-option-icon">
                        <Shield size={22} />
                      </span>
                      <span>
                        <strong>配信用表示</strong>
                        <small>名前・本文・アイコンを隠す</small>
                      </span>
                      <span className="radio-check">
                        {status.privacy && <Check size={12} strokeWidth={3} />}
                      </span>
                    </button>
                    <div className="mode-hint">
                      <ShieldCheck size={14} />
                      <span>
                        {status.privacy
                          ? "配信中も、会話の内容を画面に出しません。"
                          : "自分だけで使うときにおすすめです。"}
                      </span>
                    </div>
                  </div>
                  <div className="notification-stage">
                    <div className="stage-label">
                      <Sparkles size={12} />
                      表示プレビュー<span>サンプル</span>
                    </div>
                    <div
                      className={`notification-sample ${status.privacy ? "private" : ""}`}
                    >
                      <div className="sample-top">
                        <span>
                          <MessageCircle size={12} />
                          Discord
                        </span>
                        <small>たった今</small>
                      </div>
                      <div className="sample-message">
                        {!status.privacy && (
                          <div className="sample-avatar">S</div>
                        )}
                        <div>
                          <strong>
                            {status.privacy ? "新しい通知" : "サンプルさん"}
                          </strong>
                          {!status.privacy && <p className="sample-context">サンプルサーバー / #general</p>}
                          <p>
                            {status.privacy
                              ? "新しいメッセージがあります"
                              : "こんにちは！ VR の中でも届きます。"}
                          </p>
                        </div>
                      </div>
                    </div>
                    <div className="stage-caption">
                      {status.privacy
                        ? "送信者名・会話の場所・本文・アイコンは表示されません"
                        : "VR に表示される通知のイメージです"}
                    </div>
                  </div>
                </div>
                <div className="display-caution">
                  <Info size={14} />
                  切り替え前に送信した通知や、送信中の通知は消去できません。
                </div>
              </section>
              <div className="test-card">
                <span className="test-icon">
                  <Bell size={21} />
                </span>
                <div>
                  <h3>VR 内の表示を確認</h3>
                  <p>
                    {status.debugEnabled
                      ? "VR の表示テストは、-debug を付けずに起動してください。"
                      : status.overlay
                      ? "会話を含まないテスト通知を送れます。"
                      : "XSOverlay に接続すると、テスト通知を送れます。"}
                  </p>
                </div>
                <button
                  className="primary"
                  disabled={status.debugEnabled || !status.overlay || status.paused || busy}
                  onClick={() =>
                    void action(
                      "send_test",
                      "送信待ちに追加しました。VR 内の表示を確認してください。",
                    )
                  }
                >
                  <Bell size={15} />
                  テスト通知を送る
                  <ArrowRight size={15} />
                </button>
              </div>
              <div className="workspace-note">
                <Info size={14} />
                {status.tray
                  ? "×でトレイに収納。通知の転送はそのまま続きます。"
                  : "トレイを利用できません。画面を開いたまま使用してください。"}
              </div>
            </>
          ) : (
            <>
              <div className="page-heading settings-heading">
                <div>
                  <div className="eyebrow">MAKE IT YOURS</div>
                  <h1>{status.configured ? "設定" : "はじめの接続設定"}</h1>
                  <p>通知の見え方と、Discord への接続を整えます。</p>
                </div>
                <span className="heading-icon">
                  <Settings2 size={27} strokeWidth={1.5} />
                </span>
              </div>
              <div
                className="settings-tabs"
                role="tablist"
                aria-label="設定の項目"
              >
                {tabs.map((item) => (
                  <button
                    key={item.id}
                    role="tab"
                    aria-selected={tab === item.id}
                    aria-controls={`panel-${item.id}`}
                    id={`tab-${item.id}`}
                    className={tab === item.id ? "selected" : ""}
                    onClick={() => {
                      setTab(item.id);
                      setFormError(null);
                    }}
                  >
                    <item.icon size={16} />
                    {item.label}
                  </button>
                ))}
              </div>
              <form
                ref={formRef}
                onSubmit={save}
                noValidate
                className="settings-form"
              >
                {!form ? (
                  <div className="loading-card">設定を読み込んでいます…</div>
                ) : (
                  <>
                    <section
                      className="settings-card"
                      role="tabpanel"
                      id="panel-whitelist"
                      aria-labelledby="tab-whitelist"
                      hidden={tab !== "whitelist"}
                    >
                      <div className="section-heading">
                        <div>
                          <h2>受信を許可するチャンネル・DM</h2>
                          <p>必要な会話だけを登録します。最大32件です。</p>
                        </div>
                        <ListFilter size={21} />
                      </div>
                      <div className="whitelist-content">
                        <div className="security-note">
                          <Info size={18} />
                          <div>
                            <p>
                              登録した会話のメッセージを直接受信します。初回は
                              Discord でメッセージの読み取り権限の追加承認が必要です。
                              「Discord通知に従う」では、通常の通知だけを転送します。
                            </p>
                            <p>
                              Discord
                              側でミュートした会話も、許可リストだけ受信する間は表示対象になります。必要な会話だけを登録してください。
                            </p>
                          </div>
                        </div>
                        <div className="security-note">
                          <Info size={18} />
                          <div>
                            <p>
                              Discord
                              の「詳細設定」で開発者モードを有効にし、チャンネルの右クリックから
                              ID をコピーします。
                            </p>
                            <p>
                              DM
                              はメッセージのリンクをコピーして貼り付けることもできます。リンクからチャンネル
                              ID だけを取り出し、外部へアクセスしません。
                            </p>
                          </div>
                        </div>
                        <label htmlFor="whitelistChannelIds">
                          チャンネル ID またはメッセージリンク
                        </label>
                        <div className="whitelist-add">
                          <input
                            id="whitelistChannelIds"
                            name="whitelistChannelIds"
                            placeholder="例: 123456789012345678"
                            autoComplete="off"
                            value={channelInput}
                            aria-invalid={
                              Boolean(channelError) ||
                              fieldError("whitelistChannelIds")
                            }
                            onChange={(event) => {
                              setChannelInput(event.target.value);
                              setChannelError(null);
                            }}
                            onKeyDown={(event) => {
                              if (event.key === "Enter") {
                                event.preventDefault();
                                document.getElementById("add-channel")?.click();
                              }
                            }}
                          />
                          <button
                            id="add-channel"
                            type="button"
                            className="secondary"
                            disabled={
                              busy || form.whitelistChannelIds.length >= 32
                            }
                            onClick={() => {
                              const id = channelIdFromInput(channelInput);
                              if (!id) {
                                setChannelError(
                                  "数字の ID または Discord のメッセージリンクを入力してください。",
                                );
                                document
                                  .getElementById("whitelistChannelIds")
                                  ?.focus();
                                return;
                              }
                              if (form.whitelistChannelIds.includes(id)) {
                                setChannelError(
                                  "このチャンネルは登録済みです。",
                                );
                                return;
                              }
                              change("whitelistChannelIds", [
                                ...form.whitelistChannelIds,
                                id,
                              ]);
                              setChannelInput("");
                              setChannelError(null);
                            }}
                          >
                            <Plus size={15} />
                            追加
                          </button>
                        </div>
                        {channelError && (
                          <p className="whitelist-error" role="alert">
                            {channelError}
                          </p>
                        )}
                        <div className="whitelist-count">
                          登録済み {form.whitelistChannelIds.length} / 32
                        </div>
                        {form.whitelistChannelIds.length === 0 ? (
                          <div className="whitelist-empty">
                            まだ登録されていません。
                          </div>
                        ) : (
                          <ul className="whitelist-list">
                            {form.whitelistChannelIds.map((id) => (
                              <li key={id}>
                                <MessageCircle size={16} />
                                <code>{id}</code>
                                <button
                                  type="button"
                                  className="secondary"
                                  aria-label={`${id} を削除`}
                                  onClick={() =>
                                    change(
                                      "whitelistChannelIds",
                                      form.whitelistChannelIds.filter(
                                        (entry) => entry !== id,
                                      ),
                                    )
                                  }
                                >
                                  <X size={14} />
                                  削除
                                </button>
                              </li>
                            ))}
                          </ul>
                        )}
                      </div>
                    </section>
                    <section
                      className="settings-card bot-settings"
                      role="region"
                      id="panel-bot"
                      aria-labelledby="bot-heading"
                      hidden={tab !== "whitelist"}
                    >
                      <div className="section-heading">
                        <div>
                          <h2 id="bot-heading">Bot による取り込み中の自動判別</h2>
                          <p>
                            Bot はステータスだけを監視します。メッセージの受信は
                            RPC です。
                          </p>
                        </div>
                        <ShieldCheck size={21} />
                      </div>
                      <div className="whitelist-content">
                        <ol className="bot-guide">
                          <li>
                            Developer Portal の Bot ページで{" "}
                            <strong>Presence Intent</strong>{" "}
                            を有効にします。Message Content Intent は不要です。
                          </li>
                          <li>
                            OAuth2 の URL Generator で <strong>bot</strong>{" "}
                            を選び、権限は <strong>0</strong>{" "}
                            のまま、自分と同じサーバーへ追加します。自分専用の小さいサーバーでも使えます。
                          </li>
                          <li>
                            Bot Token、サーバー ID、自分のユーザー ID
                            を下に入力します。ID
                            は開発者モードで右クリックしてコピーできます。
                          </li>
                        </ol>
                        <div className="credential-fields">
                          <div>
                            <label htmlFor="botGuildId">
                              共通サーバーの ID
                            </label>
                            <input
                              id="botGuildId"
                              name="botGuildId"
                              inputMode="numeric"
                              maxLength={20}
                              autoComplete="off"
                              value={form.botGuildId}
                              aria-invalid={fieldError("botGuildId")}
                              onChange={(e) =>
                                change("botGuildId", e.target.value)
                              }
                            />
                            <small>
                              Bot と自分がどちらも参加しているサーバー
                            </small>
                          </div>
                          <div>
                            <label htmlFor="botUserId">自分のユーザー ID</label>
                            <input
                              id="botUserId"
                              name="botUserId"
                              inputMode="numeric"
                              maxLength={20}
                              autoComplete="off"
                              value={form.botUserId}
                              aria-invalid={fieldError("botUserId")}
                              onChange={(e) =>
                                change("botUserId", e.target.value)
                              }
                            />
                            <small>
                              RPC でログインしている自分のアカウント
                            </small>
                          </div>
                          <div className="full-width">
                            <label htmlFor="botToken">
                              Bot Token{" "}
                              {form.botTokenSet && !form.clearBotToken && (
                                <span className="configured-tag">
                                  <LockKeyhole size={12} />
                                  設定済み
                                </span>
                              )}
                            </label>
                            <input
                              id="botToken"
                              name="botToken"
                              type="password"
                              autoComplete="new-password"
                              spellCheck={false}
                              maxLength={512}
                              value={form.botToken}
                              placeholder={
                                form.botTokenSet && !form.clearBotToken
                                  ? "変更する場合だけ入力してください"
                                  : "Bot ページで発行した Token"
                              }
                              aria-invalid={fieldError("botToken")}
                              onChange={(e) =>
                                change("botToken", e.target.value)
                              }
                            />
                            <small>
                              空欄なら現在の Token
                              を維持します。暗号化して保存し、次回は画面に読み戻しません。
                            </small>
                          </div>
                        </div>
                        <div className="bot-forget">
                          <button
                            type="button"
                            className="secondary"
                            disabled={!form.botTokenSet || form.clearBotToken}
                            onClick={() => {
                              change("clearBotToken", true);
                              change("botToken", "");
                              change("receiveMode", "normal");
                            }}
                          >
                            保存済み Bot Token を削除
                          </button>
                          {form.clearBotToken && (
                            <span>
                              保存すると Token を削除し、通常受信に戻ります。
                            </span>
                          )}
                        </div>
                        <label htmlFor="receiveMode">受信モード</label>
                        <select
                          id="receiveMode"
                          name="receiveMode"
                          value={form.receiveMode}
                          onChange={(e) =>
                            change("receiveMode", e.target.value as ReceiveMode)
                          }
                        >
                          {receiveChoices.map((choice) => (
                            <option key={choice.mode} value={choice.mode}>
                              {choice.label}
                            </option>
                          ))}
                        </select>
                        <div className="security-note">
                          <Shield size={18} />
                          <div>
                            <p>
                              自動判別できないときは許可リストだけ受信します。オフライン・Invisible
                              は Bot
                              から区別できないため、制限を維持します。ホームからいつでも手動で上書きできます。
                            </p>
                          </div>
                        </div>
                      </div>
                    </section>
                    <section
                      className="settings-card"
                      role="tabpanel"
                      id="panel-display"
                      aria-labelledby="tab-display"
                      hidden={tab !== "display"}
                    >
                      <div className="section-heading">
                        <div>
                          <h2>画面と通知の見え方</h2>
                          <p>アプリのテーマと VR 内の表示を調整します。</p>
                        </div>
                        <Monitor size={21} />
                      </div>
                      <div className="setting-row">
                        <div>
                          <label htmlFor="theme">アプリのテーマ</label>
                          <p id="theme-help">システムは OS のライト・ダーク設定に従います。変更は「保存」で確定します。</p>
                        </div>
                        <select id="theme" name="theme" className="theme-select"
                          aria-describedby="theme-help" value={form.theme}
                          onChange={event => change("theme", event.target.value as ThemePreference)}>
                          <option value="system">システム（初期設定）</option>
                          <option value="light">ライト</option>
                          <option value="dark">ダーク</option>
                        </select>
                      </div>
                      {numberField(
                        "notificationTimeout",
                        "表示時間",
                        "通知を表示する長さ。1 ～ 30 秒で調整できます。",
                        1,
                        30,
                        "秒",
                      )}
                      {numberField(
                        "maxTitleLength",
                        "タイトルの文字数",
                        "長いタイトルは末尾を省略します。10 ～ 200 文字。",
                        10,
                        200,
                        "文字",
                      )}
                      {numberField(
                        "maxContentLength",
                        "本文の文字数",
                        "メッセージを読みやすい長さに。10 ～ 2000 文字。",
                        10,
                        2000,
                        "文字",
                      )}
                      <div className="setting-row">
                        <div>
                          <label htmlFor="privacyMode">配信用表示</label>
                          <p>送信者名・サーバー名・チャンネル名・本文・アイコンを隠します。</p>
                        </div>
                        <button
                          type="button"
                          id="privacyMode"
                          role="switch"
                          aria-checked={form.privacyMode}
                          className={`switch ${form.privacyMode ? "on" : ""}`}
                          onClick={() =>
                            change("privacyMode", !form.privacyMode)
                          }
                        >
                          <span />
                        </button>
                      </div>
                    </section>
                    <section
                      className="settings-card"
                      role="tabpanel"
                      id="panel-sound"
                      aria-labelledby="tab-sound"
                      hidden={tab !== "sound"}
                    >
                      <div className="section-heading">
                        <div>
                          <h2>通知音・動作</h2>
                          <p>音と、VR を終了したときの動作を調整します。</p>
                        </div>
                        <Volume2 size={21} />
                      </div>
                      <div className="sound-section">
                        <label>通知音</label>
                        <div className="sound-options">
                          {[
                            {
                              value: "",
                              title: "無音",
                              text: "静かに通知を受け取る",
                            },
                            {
                              value: "default",
                              title: "標準の通知音",
                              text: "XSOverlay の通知音",
                            },
                            {
                              value: "file",
                              title: "音声ファイル",
                              text: "好きな音を選択",
                            },
                          ].map((option) => {
                            const selected =
                              option.value === "file"
                                ? !["", "default"].includes(
                                    form.notificationSound,
                                  )
                                : form.notificationSound === option.value;
                            return (
                              <button
                                key={option.value}
                                type="button"
                                className={selected ? "selected" : ""}
                                aria-pressed={selected}
                                onClick={() =>
                                  change(
                                    "notificationSound",
                                    option.value === "file"
                                      ? "file"
                                      : option.value,
                                  )
                                }
                              >
                                <span className="sound-choice-mark">
                                  {selected ? (
                                    <Check size={13} />
                                  ) : (
                                    <Volume2 size={14} />
                                  )}
                                </span>
                                <strong>{option.title}</strong>
                                <small>{option.text}</small>
                              </button>
                            );
                          })}
                        </div>
                        {!["", "default"].includes(form.notificationSound) && (
                          <div className="file-field">
                            <input
                              name="notificationSound"
                              aria-label="通知音のファイル"
                              readOnly
                              value={
                                form.notificationSound === "file"
                                  ? ""
                                  : form.notificationSound
                              }
                              placeholder="音声ファイルを選んでください"
                            />
                            <button
                              type="button"
                              className="secondary"
                              onClick={async () => {
                                try {
                                  const path = await api<string | null>(
                                    "browse_sound",
                                  );
                                  if (path) change("notificationSound", path);
                                } catch (e) {
                                  notify(errorMessage(e), true);
                                }
                              }}
                            >
                              <FolderOpen size={15} />
                              参照
                            </button>
                          </div>
                        )}
                      </div>
                      <div className="setting-row">
                        <div>
                          <label htmlFor="volumePercent">音量</label>
                          <p>
                            {form.notificationSound
                              ? "通知音を鳴らすには 1% 以上にしてください。"
                              : "現在は無音が選択されています。"}
                          </p>
                        </div>
                        <div className="volume-control">
                          <input
                            type="range"
                            id="volumePercent"
                            name="volumePercent"
                            min="0"
                            max="100"
                            value={form.volumePercent}
                            disabled={!form.notificationSound}
                            onChange={(e) =>
                              change("volumePercent", Number(e.target.value))
                            }
                          />
                          <output>{form.volumePercent}%</output>
                        </div>
                      </div>
                      <div className="setting-row">
                        <div>
                          <label htmlFor="autoExitWithVr">
                            VR と一緒に終了
                          </label>
                          <p>
                            一度検知した VR
                            関連アプリがすべて終了すると停止します。
                          </p>
                        </div>
                        <button
                          type="button"
                          id="autoExitWithVr"
                          role="switch"
                          aria-checked={form.autoExitWithVr}
                          className={`switch ${form.autoExitWithVr ? "on" : ""}`}
                          onClick={() =>
                            change("autoExitWithVr", !form.autoExitWithVr)
                          }
                        >
                          <span />
                        </button>
                      </div>
                    </section>
                    <section
                      className="settings-card connection-settings"
                      role="tabpanel"
                      id="panel-discord"
                      aria-labelledby="tab-discord"
                      hidden={tab !== "discord"}
                    >
                      <div className="section-heading">
                        <div>
                          <h2>自分の Discord アプリを接続</h2>
                          <p>
                            利用する本人のアカウントで作成したアプリを使います。
                          </p>
                        </div>
                        <MessageCircle size={21} />
                      </div>
                      <div className="setup-guide">
                        <div>
                          <span className="step">1</span>
                          <p>
                            Developer Portal で <strong>New Application</strong>{" "}
                            を作成
                          </p>
                        </div>
                        <div>
                          <span className="step">2</span>
                          <p>
                            <strong>OAuth2</strong> で Client ID と Client
                            Secret を確認
                          </p>
                        </div>
                        <div>
                          <span className="step">3</span>
                          <p>
                            <strong>Redirects</strong> に下の URI を追加し、Save
                            Changes
                          </p>
                        </div>
                        <button
                          type="button"
                          className="portal-button"
                          onClick={() => void action("open_portal")}
                        >
                          Developer Portal を開く
                          <ExternalLink size={13} />
                        </button>
                      </div>
                      <div className="credential-fields">
                        <div>
                          <label htmlFor="clientId">Client ID</label>
                          <input
                            id="clientId"
                            name="clientId"
                            inputMode="numeric"
                            autoComplete="off"
                            value={form.clientId}
                            placeholder="例：123456789012345678"
                            aria-invalid={fieldError("clientId")}
                            onChange={(e) => change("clientId", e.target.value)}
                          />
                          <small>OAuth2 に表示されている数字の ID</small>
                        </div>
                        <div>
                          <label htmlFor="clientSecret">
                            Client Secret
                            {form.secretSet && (
                              <span className="saved-secret">
                                <LockKeyhole size={11} />
                                設定済み
                              </span>
                            )}
                          </label>
                          <input
                            id="clientSecret"
                            name="clientSecret"
                            type="password"
                            autoComplete="new-password"
                            value={form.clientSecret}
                            placeholder={
                              form.secretSet
                                ? "変更する場合だけ入力してください"
                                : "自分のアプリの Secret を入力"
                            }
                            aria-invalid={fieldError("clientSecret")}
                            onChange={(e) =>
                              change("clientSecret", e.target.value)
                            }
                          />
                          <small>
                            {form.secretSet
                              ? "空欄で保存すると、現在の Secret を引き継ぎます。"
                              : "Bot Token やアカウントのトークンは入力しません。"}
                          </small>
                        </div>
                        <div className="redirect-field">
                          <label htmlFor="redirectUri">Redirect URI</label>
                          <input
                            id="redirectUri"
                            name="redirectUri"
                            autoComplete="off"
                            value={form.redirectUri}
                            aria-invalid={fieldError("redirectUri")}
                            onChange={(e) =>
                              change("redirectUri", e.target.value)
                            }
                          />
                          <small>
                            Portal の Redirects に登録した URL
                            と一致させてください。
                          </small>
                        </div>
                      </div>
                      <div className="security-note">
                        <ShieldCheck size={17} />
                        <p>
                          Secret は Windows ユーザーに紐づけて暗号化保存します。
                          <br />
                          設定ファイルは他の人と共有しないでください。
                        </p>
                      </div>
                    </section>
                    {formError && (
                      <div className="error-banner" role="alert">
                        <CircleHelp size={18} />
                        {formError.message}
                      </div>
                    )}
                    <div className="settings-footer">
                      <span>
                        <Info size={14} />
                        設定中は通知の転送を一時停止しています。
                      </span>
                      <div>
                        <button
                          type="button"
                          className="secondary"
                          onClick={() => void cancel()}
                          disabled={busy}
                        >
                          キャンセル
                        </button>
                        <button
                          type="submit"
                          className="primary"
                          disabled={busy}
                        >
                          <Check size={16} />
                          {busy ? "保存中…" : "保存して戻る"}
                        </button>
                      </div>
                    </div>
                  </>
                )}
              </form>
            </>
          )}
          <footer className="page-footer">
            <span>Discord to VR</span>
            <span>いつもの通知を、いつものまま。</span>
          </footer>
        </main>
      </div>
      {toast && (
        <div
          className={`toast ${toast.error ? "error" : ""}`}
          role={toast.error ? "alert" : "status"}
        >
          {toast.error ? <CircleHelp size={18} /> : <Check size={18} />}
          <p>{toast.message}</p>
          <button aria-label="通知を閉じる" onClick={() => setToast(null)}>
            <X size={16} />
          </button>
        </div>
      )}
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
