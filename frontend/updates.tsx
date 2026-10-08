import { useEffect, useState } from "react";
import { Download, RefreshCw, ShieldCheck, Info, CheckCheck } from "lucide-react";
import { api, errorMessage } from "./bridge";
import "./updates.css";

export interface UpdateStatus {
  currentVersion: string;
  autoUpdate: boolean;
  supported: boolean;
  canCheck: boolean;
  disabledReason: string | null;
  phase: "idle" | "checking" | "current" | "available" | "downloading" | "waiting" | "installing" | "error";
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  checkedAtMs: number | null;
  message: string | null;
}
export function useUpdateStatus() {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try { const next=await api<UpdateStatus>("get_update_status"); if(active)setStatus(next); }
      catch { /* Retried; actions show their own error. */ }
    };
    void refresh();
    const timer=setInterval(()=>void refresh(),1000);
    return ()=>{active=false;clearInterval(timer);};
  },[]);
  return status;
}
const labels = {
  idle: "更新の確認を待っています", checking: "新しいバージョンを確認中…",
  current: "最新のバージョンです", available: "新しいバージョンがあります",
  downloading: "更新をダウンロード中…", waiting: "設定を閉じるまで更新を待機中",
  installing: "更新を適用して再起動します…", error: "更新を完了できませんでした",
};
const mb=(bytes:number)=>(bytes/(1024*1024)).toFixed(1);
export function UpdateNotice({status,onOpen}:{status:UpdateStatus|null;onOpen:()=>void}) {
  if(!status || !["available","downloading","waiting","installing","error"].includes(status.phase))return null;
  return <div className={`update-notice ${status.phase==="error"?"update-error":""}`} role="status"><Download size={18}/><span>{labels[status.phase]}{status.version&&` · v${status.version}`}</span><button type="button" className="update-text-button" onClick={onOpen}>更新の詳細</button></div>;
}
export function UpdatesPanel({status}:{status:UpdateStatus|null}) {
  const [working,setWorking]=useState(false);
  const [error,setError]=useState("");
  const processing=Boolean(status&&["checking","downloading","waiting","installing"].includes(status.phase));
  const blocked=!status?.canCheck;
  async function run(action:()=>Promise<unknown>) {
    setWorking(true);setError("");
    try {await action();}catch(e){setError(errorMessage(e));}finally{setWorking(false);}
  }
  const percentage=status?.total?Math.min(100,Math.floor(status.downloaded/status.total*100)):undefined;
  return <div className="updates-page">
    <div className="page-heading"><div><div className="eyebrow">APP UPDATES</div><h1>アプリの更新</h1><p>新しいバージョンを自動でダウンロード・適用します。</p></div><span className="heading-icon"><Download size={27}/></span></div>
    <section className="settings-card update-card">
      <div className="section-heading"><div><h2>現在のバージョン</h2><p>Discord to VR <strong>v{status?.currentVersion??"読み込み中"}</strong></p></div><ShieldCheck size={24}/></div>
      <div className="update-card-body">
        {status?.disabledReason&&<p className="update-info"><Info size={18}/>{status.disabledReason}</p>}
        <label className="update-toggle"><input type="checkbox" checked={status?.autoUpdate??true} disabled={!status||working||blocked||status.phase==="installing"} onChange={e=>void run(()=>api("set_auto_update",{enabled:e.target.checked}))}/><span><strong>自動アップデート</strong><small>起動後と12時間ごとに確認し、新版があればダウンロード・署名検証・適用・再起動まで自動で行います。</small></span></label>
        <p className="update-info"><Info size={18}/>設定の入力中はダウンロード後に待機します。設定を保存するか、閉じると更新を適用します。通知の転送は適用・再起動の間だけ停止します。</p>
        <div className="update-state" aria-live="polite"><CheckCheck size={20}/><div><strong>{status?labels[status.phase]:"更新情報を読み込み中…"}</strong>{status?.version&&<p>更新先: v{status.version}</p>}{status?.checkedAtMs&&<small>最終確認: {new Date(status.checkedAtMs).toLocaleString("ja-JP")}</small>}</div></div>
        {status?.phase==="downloading"&&<div className="update-progress"><progress max={100} value={percentage} aria-label="更新のダウンロード"/><span>{mb(status.downloaded)} MB{status.total?` / ${mb(status.total)} MB`:""}{percentage!==undefined&&`（${percentage}%）`}</span></div>}
        {(error||status?.message)&&<div className="error-banner" role="alert"><Info size={18}/>{error||status?.message}</div>}
        <button type="button" className="secondary" disabled={!status||blocked||processing||working} onClick={()=>void run(()=>api("check_updates"))}><RefreshCw size={16}/>今すぐ確認{status?.autoUpdate&&status.supported?"して更新":""}</button>
        {status?.notes&&<div className="update-notes"><h3>このバージョンの変更内容</h3><p>{status.notes}</p></div>}
        {status?.phase==="available"&&status.supported&&!status.autoUpdate&&<p className="update-info">自動アップデートをオンにすると、新版の適用を開始します。</p>}
        <p className="update-security"><ShieldCheck size={17}/>配布元の署名とバージョンが一致した更新だけを適用します。確認・ダウンロード・署名検証に失敗しても、現在のアプリは変更しません。</p>
      </div>
    </section>
  </div>;
}
