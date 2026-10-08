import { useEffect, useState } from "react";
import { Activity, CheckCheck, FlaskConical, Info, RefreshCw, ShieldCheck, Trash2 } from "lucide-react";
import { api, errorMessage, type DebugOutcome, type DebugSnapshot, type Status } from "./bridge";
import "./diagnostics.css";

const outcomes: Record<DebugOutcome, string> = {
  received_notification: "Discord 通知イベントを受信",
  received_message: "会話の直接受信イベントを受信",
  filtered_mode: "現在の受信モードで対象外",
  filtered_blocked: "ブロックした会話のため除外",
  filtered_duplicate: "同じメッセージのため重複を除外",
  filtered_self: "自分が送信したメッセージのため除外",
  filtered_empty: "表示する内容がないため除外",
  queued: "送信待ちに追加",
  validated: "画面内チェック完了（VR 送信なし）",
  sent: "XSOverlay の WebSocket へ送信",
  drop_no_overlay: "XSOverlay 未接続のため破棄",
  drop_full: "送信待ちが満杯、または送信処理が停止",
  drop_expired: "待ち時間の上限を超えて破棄",
  drop_policy: "受信モードが変わったため送信前に破棄",
  drop_disconnected: "切断時に送信待ちを破棄",
  drop_send_failed: "XSOverlay への送信に失敗",
  drop_invalid: "通知の組み立てに失敗",
  drop_stopped: "処理の停止時に送信待ちを破棄",
};
const sourceLabels = { live: "実受信", simulation: "ダミー", overlay_test: "VR テスト", system: "接続案内" };
const modeLabels = { normal: "Discord通知に従う", whitelist: "許可リストだけ", auto: "Botで自動判別" };
const botLabels = {disabled:"停止中",connecting:"接続中",unknown:"判定待ち",dnd:"取り込み中",online:"オンライン",idle:"退席中",offline:"オフライン・判定保留",error:"Token・Presence Intent を確認",missing_guild:"共通サーバーを確認",missing_user:"対象ユーザーを確認",account_mismatch:"RPC のアカウントと不一致"};
const time = (ms: number | null) => ms ? new Date(ms).toLocaleTimeString("ja-JP") : "まだ受信していません";

export function DiagnosticsPanel({status, onSetup, onWhitelist}: {status: Status; onSetup:()=>void; onWhitelist:()=>void}) {
  const [snapshot, setSnapshot] = useState<DebugSnapshot | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [raw, setRaw] = useState(false);
  const [registered, setRegistered] = useState(false);
  const [presence, setPresence] = useState("dnd");
  useEffect(() => {
    let active = true;
    async function refresh() {
      try { const result = await api<DebugSnapshot>("get_debug_snapshot"); if (active) setSnapshot(result); }
      catch(e) { if(active) setError(errorMessage(e)); }
    }
    void refresh();
    const timer = setInterval(() => void refresh(), 1000);
    return () => { active=false; clearInterval(timer); };
  }, []);
  useEffect(() => { if(!status.whitelistCount) setRegistered(false); }, [status.whitelistCount]);
  async function run(task:()=>Promise<void>) {
    setWorking(true);setError("");setNotice("");
    try { await task(); } catch(e) { setError(errorMessage(e)); }
    finally {setWorking(false);}
  }
  return <div className="debug-page">
    <div className="page-heading"><div><div className="eyebrow">DEBUG MODE</div><h1>Discord の受信診断</h1><p>XSOverlay を使わず、Discord から届いているか確認できます。</p></div><span className="heading-icon"><Activity size={27} /></span></div>
    <div className="debug-test-note"><Info size={16} /><span>受信テスト専用で起動しています。XSOverlay への接続・VR 送信・VR 終了による自動終了は行いません。VR への転送はフラグなしで起動してください。</span></div>
    <div className="debug-privacy-note"><ShieldCheck size={17} /><span>記録するのは日時・処理結果だけです。本文、送信者名、ID、認証情報は記録せず、履歴はこの起動中だけ最大100件保持します。</span></div>
    <section className="debug-card" aria-labelledby="debug-live-title">
      <div className="section-heading"><div><h2 id="debug-live-title">Discord からの実受信</h2><p>ダミーテストはこの数値に含みません。</p></div><span className={`status-label ${status.discord===2&&!status.paused?"good":""}`}>{status.paused?"設定中・停止":status.discord===2?"接続済み":status.discord===4?"認証を確認":"接続待ち"}</span></div>
      <dl className="debug-counts">{([['received','受信'],['filtered','対象外'],['queued','画面内チェック待ちに追加'],['validated','画面内チェック完了'],['dropped','破棄']] as const).map(([key,label])=><div key={key}><dt>{label}</dt><dd>{snapshot?.live[key]??0}</dd></div>)}</dl>
      <div className="debug-context"><span>最終受信: <strong>{time(snapshot?.live.lastReceivedAtMs??null)}</strong></span><span>受信モード: <strong>{modeLabels[status.receiveMode]}</strong></span><span>XSOverlay: <strong>使用しません</strong></span>{status.receiveMode==="auto"&&<span>Bot: <strong>{botLabels[status.botStatus]}</strong></span>}</div>
      <div className="debug-instructions"><ol><li>Discord が接続済みであることを確認します。XSOverlay・SteamVR は起動不要です。</li><li>この画面を開いたまま、別のアカウントなどから通知が出るメッセージを受信してください。</li><li>「受信」と履歴が増えれば、Discord からの受信成功です。「画面内チェック完了」は受信判定後の通知を組み立てられたことを示します。</li></ol><p>対象外なら履歴の除外理由、受信0件なら Discord の通知設定・取り込み中・接続設定を確認してください。自分が送信したメッセージは対象外です。許可リストだけを受信する間は登録先の会話で試してください。</p><div className="debug-links"><button type="button" className="secondary" onClick={onSetup}>{status.configured?"接続設定を確認":"初回案内を開く"}</button><button type="button" className="secondary" onClick={onWhitelist}>許可リストを確認</button></div></div>
    </section>
    <section className="debug-card" aria-labelledby="debug-test-title">
      <div className="section-heading"><div><h2 id="debug-test-title">アプリ内でダミー受信を試す</h2><p>実際と同じメッセージ処理・受信判定・通知の組み立てを確認します。</p></div><FlaskConical size={23} /></div>
      <div className="debug-card-body"><p className="debug-test-note"><Info size={16} />このテストは Discord に接続せず、VR に送信しません。Bot の仮の状態はテストだけに使い、実際の受信には反映しません。</p>
        <fieldset className="debug-test-fields" disabled={working}>
          <div><label htmlFor="debug-event">試すイベント</label><select id="debug-event" value={String(raw)} onChange={e=>setRaw(e.target.value==="true")}><option value="false">通常の Discord 通知</option><option value="true">チャンネル・DM の直接受信</option></select></div>
          <div><label htmlFor="debug-target">試す会話</label><select id="debug-target" value={String(registered)} onChange={e=>setRegistered(e.target.value==="true")}><option value="false">許可リストにない会話</option><option value="true" disabled={!status.whitelistCount}>許可リストに登録済みの会話</option></select></div>
          <div><label htmlFor="debug-presence">テスト用の Bot 判定</label><select id="debug-presence" value={presence} disabled={status.receiveMode!=="auto"} onChange={e=>setPresence(e.target.value)}><option value="dnd">取り込み中</option><option value="online">オンライン</option><option value="offline">オフライン</option><option value="unknown">状態不明</option></select><small>自動判別モードのテストで使います。</small></div>
        </fieldset>
        <button type="button" className="primary" disabled={working} onClick={()=>void run(async()=>{setSnapshot(await api<DebugSnapshot>("run_debug_receive_test",{raw,registered,presence}));setNotice("ダミー受信を実行しました。下のテスト結果と履歴を確認してください。");})}><FlaskConical size={16} />ダミー受信を試す</button>
        <dl className="debug-test-counts"><div><dt>ダミー受信</dt><dd>{snapshot?.tests.received??0}</dd></div><div><dt>画面内チェック完了</dt><dd>{snapshot?.tests.validated??0}</dd></div><div><dt>テストで対象外</dt><dd>{snapshot?.tests.filtered??0}</dd></div><div><dt>テストで破棄</dt><dd>{snapshot?.tests.dropped??0}</dd></div></dl>
      </div>
    </section>
    {error&&<div className="error-banner" role="alert"><Info size={18}/>{error}</div>}{notice&&<p className="debug-result" role="status"><CheckCheck size={17}/>{notice}</p>}
    <section className="debug-card" aria-labelledby="debug-events-title"><div className="section-heading"><div><h2 id="debug-events-title">最近の処理</h2><p>新しいものから表示します。記録開始: {snapshot?time(snapshot.startedAtMs):"読み込み中"}</p></div><button type="button" className="secondary" disabled={working} onClick={()=>void run(async()=>{await api("clear_debug_events");setSnapshot(await api<DebugSnapshot>("get_debug_snapshot"));setNotice("受信件数と履歴をクリアしました。");})}><Trash2 size={15}/>記録をクリア</button></div>
      {!snapshot?.events.length?<div className="debug-empty"><RefreshCw size={20}/><p>まだ処理の記録がありません。ダミー受信を試すか、Discord のメッセージを受信してください。</p></div>:<ol className="debug-events" aria-label="受信処理の履歴">{snapshot.events.map(event=><li key={event.id} className={event.outcome.startsWith("drop_")?"debug-event-drop":""}><time dateTime={new Date(event.atMs).toISOString()}>{time(event.atMs)}</time><span className={`debug-source ${event.source}`}>{sourceLabels[event.source]}</span><span>{outcomes[event.outcome]}</span></li>)}</ol>}
    </section>
  </div>;
}
