import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Bell, Check, CheckCheck, ExternalLink, Gamepad2, Info, LockKeyhole, MessageCircle, Monitor, ShieldCheck, Sparkles } from "lucide-react";
import { api, errorMessage, type Settings, type SettingsInput, type Status } from "./bridge";
import "./onboarding.css";

const steps = ["ようこそ", "準備", "Discord接続", "表示を選ぶ", "接続確認"];
type Props = {
  initial: Settings;
  status: Status;
  busy: boolean;
  resumeConnection: boolean;
  onSave: (input: SettingsInput) => Promise<Settings>;
  onEdit: () => Promise<Settings>;
  onFinish: (extras?: boolean) => Promise<void>;
  onLater: () => Promise<void>;
};

export function Onboarding({ initial, status, busy, resumeConnection, onSave, onEdit, onFinish, onLater }: Props) {
  const [step, setStep] = useState(resumeConnection ? 4 : 0);
  const [values, setValues] = useState(initial);
  const [secret, setSecret] = useState("");
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<{ field?: string; message: string } | null>(null);
  const [notice, setNotice] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  const locked = busy || working;
  const ready = status.discord === 2 && (status.debugEnabled || status.overlay) && !status.paused;
  useEffect(() => {
    heading.current?.focus();
    window.scrollTo(0, 0);
  }, [step]);
  async function run(task: () => Promise<void>) {
    setWorking(true);
    setError(null);
    setNotice("");
    try { await task(); }
    catch (e) {
      const field = e && typeof e === "object" && "field" in e ? String(e.field ?? "") : "";
      setError({ field, message: errorMessage(e) });
      if (["clientId", "clientSecret", "redirectUri"].includes(field)) {
        setStep(2);
        setTimeout(() => document.getElementById(`tutorial-${field}`)?.focus(), 0);
      }
    } finally { setWorking(false); }
  }
  async function next() {
    setError(null);
    if (step === 2) {
      let field = "";
      if (!/^[0-9]+$/.test(values.clientId.trim())) field = "clientId";
      else if (!secret.trim() && !values.secretSet) field = "clientSecret";
      else {
        try {
          const url = new URL(values.redirectUri);
          if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.hash) field = "redirectUri";
        } catch { field = "redirectUri"; }
      }
      if (field) {
        setError({ field, message: field === "clientId" ? "数字の Client ID を入力してください。" : field === "clientSecret" ? "自分のアプリの Client Secret を入力してください。" : "Portal に登録した有効な Redirect URI を入力してください。" });
        document.getElementById(`tutorial-${field}`)?.focus();
        return;
      }
    }
    if (step === 3) {
      await run(async () => {
        const { secretSet: _secretSet, botTokenSet: _botTokenSet, ...settings } = values;
        const saved = await onSave({ ...settings, clientSecret: secret.trim() || null, botToken: null, clearBotToken: false });
        setSecret("");
        setValues(saved);
        setStep(4);
      });
    } else { setStep(step + 1); }
  }
  async function back() {
    if (step === 4) {
      await run(async () => {
        setValues(await onEdit());
        setSecret("");
        setStep(3);
      });
    } else { setError(null); setStep(step - 1); }
  }
  return (
    <div className="onboarding">
      <div className="page-heading">
        <div><div className="eyebrow">LET'S GET STARTED</div><h1>はじめての設定</h1><p>ひとつずつ進めて、VR に通知を届けましょう。</p></div>
        <span className="heading-icon"><Sparkles size={27} strokeWidth={1.5} /></span>
      </div>
      <ol className="tutorial-steps" aria-label="設定の進み具合">
        {steps.map((label, index) => <li key={label} className={index === step ? "current" : index < step ? "done" : ""} aria-current={index === step ? "step" : undefined}>
          <span className="tutorial-step-number">{index < step ? <Check size={15} /> : index + 1}</span><span>{label}</span>
        </li>)}
      </ol>
      <section className="tutorial-card" aria-labelledby="tutorial-heading">
        <div className="tutorial-step-label">STEP {step + 1} / {steps.length}</div>
        <h2 id="tutorial-heading" ref={heading} tabIndex={-1}>{["いつもの Discord を、VR の中へ。", "使うアプリを準備しましょう", "自分の Discord アプリを作りましょう", "通知の見え方を選びましょう", "保存できました。接続を確認しましょう"][step]}</h2>
        {step === 0 && <>
          <p className="tutorial-lead">{status.debugEnabled ? "今回は Discord の受信テスト専用です。XSOverlay・SteamVR は起動せず、Discord の接続設定を進めます。" : "Discord のメッセージ通知を XSOverlay に届けます。まずは通常の通知を受け取れるように設定します。"}</p>
          <div className="tutorial-flow" aria-label="Discord の通知をこのアプリから XSOverlay へ転送">
            <div><span className="discord-icon"><MessageCircle size={28} /></span><strong>Discord</strong><small>通知を受け取る</small></div><ArrowRight className="tutorial-flow-arrow" size={20} />
            <div><span className="tutorial-app-icon"><Bell size={28} /></span><strong>Discord to VR</strong><small>この PC で転送</small></div><ArrowRight className="tutorial-flow-arrow" size={20} />
            <div><span className="overlay-icon"><Gamepad2 size={28} /></span><strong>XSOverlay</strong><small>VR の中に表示</small></div>
          </div>
          <div className="tutorial-callout"><ShieldCheck size={20} /><div><strong>自分のアカウントで使います</strong><p>自分の Client ID と Client Secret を用意します。保存する秘密情報は、この Windows ユーザーで暗号化して保護します。</p></div></div>
          <p className="tutorial-muted">取り込み中の許可リストや Bot 自動判別は、基本設定が終わってから追加できます。</p>
        </>}
        {step === 1 && <>
          <p className="tutorial-lead">この3つを用意してください。今起動できなくても、接続確認はあとで行えます。</p>
          <div className="tutorial-preparations">
            <article><Monitor size={23} /><div><h3>Discord デスクトップアプリ</h3><p>通知を受け取る自分のアカウントでログインします。Windows のアプリ版を使ってください。</p></div></article>
            <article><Gamepad2 size={23} /><div><h3>SteamVR と XSOverlay</h3><p>{status.debugEnabled ? "受信テストでは起動不要です。VR への転送を試すときに、フラグなしで起動してください。" : "両方を起動し、XSOverlay の通知用 WebSocket を有効にします。"}</p></div></article>
            <article><LockKeyhole size={23} /><div><h3>自分の Discord アプリ</h3><p>次の画面で Developer Portal を開き、無料で作成します。Bot は基本設定には必要ありません。</p></div></article>
          </div>
          <div className="tutorial-callout"><Info size={20} /><p>ふだんは Discord が表示する通知を転送します。取り込み中も必要な会話を受け取りたい場合は、あとで許可リストを設定できます。</p></div>
        </>}
        {step === 2 && <>
          <p className="tutorial-lead">ブラウザーでアプリを作り、2つの値をこの画面へコピーします。</p>
          <button type="button" className="secondary tutorial-portal" disabled={locked || status.preview} onClick={() => void run(async () => { await api("open_portal"); })}><ExternalLink size={16} />Developer Portal を開く</button>
          <ol className="tutorial-instructions">
            <li><strong>自分のアカウントで「New Application」</strong><p>Discord にログインしている本人のアカウントを使います。アプリ名は自由です。</p></li>
            <li><strong>「OAuth2」で Redirects を登録</strong><p><code>{values.redirectUri || "http://localhost/"}</code> を追加し、「Save Changes」で保存します。</p></li>
            <li><strong>Client ID と Client Secret をコピー</strong><p>OAuth2 ページの値を下へ貼り付けます。Secret が見えない場合は「Reset Secret」で発行します。</p></li>
          </ol>
          <fieldset disabled={locked} className="tutorial-fields">
            <div><label htmlFor="tutorial-clientId">Client ID</label><input id="tutorial-clientId" inputMode="numeric" autoComplete="off" value={values.clientId} placeholder="数字の ID を貼り付け" aria-invalid={error?.field === "clientId"} onChange={e => setValues({ ...values, clientId: e.target.value })} /></div>
            <div><label htmlFor="tutorial-clientSecret">Client Secret {values.secretSet && <span className="configured-tag"><Check size={12} />設定済み</span>}</label><input id="tutorial-clientSecret" type="password" autoComplete="new-password" spellCheck={false} value={secret} placeholder={values.secretSet ? "変更しなければ空欄で進めます" : "自分のアプリの Secret を貼り付け"} aria-invalid={error?.field === "clientSecret"} onChange={e => setSecret(e.target.value)} /><small>Bot Token や Discord アカウントのトークンは入力しません。</small></div>
            <div className="tutorial-redirect"><label htmlFor="tutorial-redirectUri">Redirect URI</label><input id="tutorial-redirectUri" autoComplete="off" value={values.redirectUri} aria-invalid={error?.field === "redirectUri"} onChange={e => setValues({ ...values, redirectUri: e.target.value })} /><small>Portal に登録した URL と完全に同じ値にします。通常はこのままで使えます。</small></div>
          </fieldset>
          <div className="tutorial-callout"><LockKeyhole size={19} /><p>保存済みの Secret は画面に読み戻しません。変更しない場合は空欄のまま進めます。</p></div>
        </>}
        {step === 3 && <>
          <p className="tutorial-lead">ホームでいつでも切り替えられます。ほかの表示・通知音の設定も、あとで変更できます。</p>
          <div className="tutorial-display-options">
            {[false, true].map(privacy => <button type="button" key={String(privacy)} disabled={locked} aria-pressed={values.privacyMode === privacy} className={`tutorial-display-option ${values.privacyMode === privacy ? "selected" : ""}`} onClick={() => setValues({ ...values, privacyMode: privacy })}>
              {privacy ? <ShieldCheck size={23} /> : <MessageCircle size={23} />}<strong>{privacy ? "配信用表示" : "通常表示"}</strong><p>{privacy ? "名前・本文・アイコンを隠す" : "誰からのメッセージか確認できる"}</p><span>{values.privacyMode === privacy ? <CheckCheck size={17} /> : null}{values.privacyMode === privacy ? "選択中" : "この表示を選ぶ"}</span>
            </button>)}
          </div>
          <div className="tutorial-sample"><span>VR での表示例（ダミー）</span><strong>{values.privacyMode ? "Discord" : "サンプルさん"}</strong><p>{values.privacyMode ? "新しいメッセージがあります" : "こんにちは！ VR の中でも届きます。"}</p></div>
          <div className="tutorial-callout"><Info size={19} /><p>配信用表示へ切り替えても、すでに表示した通知は消えません。配信を始める前に選んでください。</p></div>
          <p className="tutorial-muted">次へ進むと接続設定を保存します。Discord の承認画面が開いたら、自分のアプリであることを確認して承認してください。</p>
        </>}
        {step === 4 && <>
          <p className="tutorial-lead">{status.debugEnabled ? "Discord の承認画面を確認してください。接続済みになったら、サイドバーの「受信診断」で実際のメッセージを受け取って確認できます。XSOverlay は使いません。" : "Discord の承認画面を確認してください。Discord と XSOverlay が接続済みになったら、VR 内の表示を試せます。"}</p>
          <div className="tutorial-connection-list" aria-live="polite">
            <article><MessageCircle size={23} /><div><strong>Discord</strong><p>{status.discord === 2 ? "通知を受信する準備ができました。" : status.discord === 4 ? "Client ID・Secret・Redirect URI と承認状態を確認してください。「戻る」から修正できます。" : status.discord === 1 ? "Discord に表示された承認画面を確認してください。" : "Discord デスクトップアプリを起動して待ってください。"}</p></div><span className={`status-label ${status.discord === 2 ? "good" : ""}`}>{status.discord === 2 ? "接続済み" : status.discord === 4 ? "確認が必要" : "接続待ち"}</span></article>
            <article><Gamepad2 size={23} /><div><strong>XSOverlay</strong><p>{status.debugEnabled ? "受信テストでは接続・VR 送信を行いません。" : status.overlay ? "VR に通知を送る準備ができました。" : "SteamVR と XSOverlay を起動し、通知用 WebSocket を有効にしてください。"}</p></div><span className={`status-label ${status.overlay ? "good" : ""}`}>{status.debugEnabled ? "使用しません" : status.overlay ? "接続済み" : "接続待ち"}</span></article>
          </div>
          {!status.debugEnabled && <button type="button" className="secondary" disabled={locked || !status.overlay || status.paused || status.preview} onClick={() => void run(async () => { await api("send_test"); setNotice("テスト通知を送信待ちに追加しました。VR 内に表示されることを確認してください。"); })}><Bell size={16} />テスト通知を送る</button>}
          {notice && <p className="tutorial-notice" role="status">{notice}</p>}
          <div className="tutorial-callout"><Info size={20} /><div><strong>取り込み中にも受け取りたい場合</strong><p>許可リストにチャンネル・DM を登録します。手動で切り替えるほか、Bot で取り込み中を自動判別できます。</p><button type="button" className="tutorial-text-button" disabled={locked} onClick={() => void run(() => onFinish(true))}>許可リスト・Bot の設定へ進む<ArrowRight size={15} /></button></div></div>
          <p className="tutorial-muted">×・Esc はトレイへ収納します。転送を止める場合は「アプリを終了」を選びます。接続確認はホームからも行えます。</p>
        </>}
        {error && <div className="tutorial-error" role="alert"><Info size={18} />{error.message}</div>}
        <div className="tutorial-footer">
          <button type="button" className="secondary" disabled={locked || step === 0} onClick={() => void back()}><ArrowLeft size={15} />戻る</button>
          <button type="button" className="tutorial-text-button" disabled={locked} onClick={() => void run(onLater)}>{step === 4 ? "ホームであとから確認" : "あとで設定する"}</button>
          <button type="button" className="primary" disabled={locked} onClick={() => void (step === 4 ? run(() => onFinish()) : next())}>{locked ? "処理中…" : ["設定をはじめる", "次へ", "次へ", "保存して接続を確認", ready ? "完了してホームへ" : "接続はあとで・ホームへ"][step]}<ArrowRight size={16} /></button>
        </div>
      </section>
    </div>
  );
}
