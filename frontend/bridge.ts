import { invoke, isTauri } from "@tauri-apps/api/core";
export type ReceiveMode = "normal" | "whitelist" | "auto";
export type ThemePreference = "system" | "light" | "dark";
export type BotStatus =
  | "disabled"
  | "connecting"
  | "unknown"
  | "dnd"
  | "online"
  | "idle"
  | "offline"
  | "error"
  | "missing_guild"
  | "missing_user"
  | "account_mismatch";
export interface Status {
  theme: ThemePreference;
  discord: number;
  overlay: boolean;
  privacy: boolean;
  configured: boolean;
  paused: boolean;
  tray: boolean;
  problem: string | null;
  preview: boolean;
  startSettings: boolean;
  receiveMode: ReceiveMode;
  whitelistOnly: boolean;
  whitelistCount: number;
  whitelistStatus: number;
  botStatus: BotStatus;
  botConfigured: boolean;
  onboardingPending: boolean;
  debugEnabled: boolean;
}
export type DebugOutcome = "received_notification" | "received_message" | "filtered_mode" | "filtered_blocked" | "filtered_duplicate" | "filtered_self" | "filtered_empty" | "queued" | "validated" | "sent" | "drop_no_overlay" | "drop_full" | "drop_expired" | "drop_policy" | "drop_disconnected" | "drop_send_failed" | "drop_invalid" | "drop_stopped";
export interface DebugCounts { received: number; filtered: number; queued: number; validated: number; sent: number; dropped: number; lastReceivedAtMs: number | null; }
export interface DebugSnapshot { startedAtMs: number; live: DebugCounts; tests: DebugCounts; events: {id: number; atMs: number; source: "live" | "simulation" | "overlay_test" | "system"; outcome: DebugOutcome;}[]; }
export interface Settings {
  theme: ThemePreference;
  clientId: string;
  secretSet: boolean;
  redirectUri: string;
  notificationTimeout: number;
  volumePercent: number;
  maxTitleLength: number;
  maxContentLength: number;
  notificationSound: string;
  privacyMode: boolean;
  autoExitWithVr: boolean;
  whitelistChannelIds: string[];
  receiveMode: ReceiveMode;
  botTokenSet: boolean;
  botGuildId: string;
  botUserId: string;
}
export type SettingsInput = Omit<Settings, "secretSet" | "botTokenSet"> & {
  clientSecret: string | null;
  botToken: string | null;
  clearBotToken: boolean;
};
export const desktop = isTauri();
export function channelIdFromInput(text: string): string | null {
  let id = text.trim();
  if (!/^\d+$/.test(id)) {
    try {
      const url = new URL(id);
      if (
        url.protocol !== "https:" ||
        !["discord.com", "canary.discord.com", "ptb.discord.com"].includes(
          url.hostname,
        ) ||
        url.username ||
        url.password
      )
        return null;
      const match = url.pathname.match(
        /^\/channels\/(?:@me|\d+)\/(\d+)(?:\/\d+)?\/?$/,
      );
      if (!match) return null;
      id = match[1];
    } catch {
      return null;
    }
  }
  if (!/^[1-9]\d{0,19}$/.test(id) || BigInt(id) > 18446744073709551615n)
    return null;
  return id;
}
const previewStatus: Status = {
  theme: "system",
  discord: 2,
  overlay: true,
  privacy: false,
  configured: true,
  paused: false,
  tray: true,
  problem: null,
  preview: true,
  startSettings: false,
  receiveMode: "normal",
  whitelistOnly: false,
  whitelistCount: 0,
  whitelistStatus: 0,
  botStatus: "disabled",
  botConfigured: false,
  onboardingPending: false,
  debugEnabled: !desktop && new URLSearchParams(window.location.search).has("debug"),
};
let previewSettings: Settings = {
  theme: "system",
  clientId: "123456789012345678",
  secretSet: true,
  redirectUri: "http://localhost/",
  notificationTimeout: 5,
  volumePercent: 0,
  maxTitleLength: 35,
  maxContentLength: 70,
  notificationSound: "",
  privacyMode: false,
  autoExitWithVr: true,
  whitelistChannelIds: [],
  receiveMode: "normal",
  botTokenSet: false,
  botGuildId: "",
  botUserId: "",
};
const emptyDebugCounts = (): DebugCounts => ({received:0,filtered:0,queued:0,validated:0,sent:0,dropped:0,lastReceivedAtMs:null});
let previewDebug: DebugSnapshot = {startedAtMs:Date.now(),live:emptyDebugCounts(),tests:emptyDebugCounts(),events:[]};
let previewDebugId = 0;
let previewAutoUpdate = true;
function previewDebugEvent(outcome: DebugOutcome) {
  const atMs=Date.now();
  previewDebug.events.unshift({id:++previewDebugId,atMs,source:"simulation",outcome});
  previewDebug.events=previewDebug.events.slice(0,100);
  if(outcome.startsWith("received_")){previewDebug.tests.received++;previewDebug.tests.lastReceivedAtMs=atMs;}
  else if(outcome==="filtered_mode")previewDebug.tests.filtered++;
  else if(outcome==="queued")previewDebug.tests.queued++;
  else if(outcome==="validated")previewDebug.tests.validated++;
}
export async function api<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (desktop) return invoke<T>(command, args);
  if (["get_debug_snapshot","clear_debug_events","run_debug_receive_test"].includes(command) && !previewStatus.debugEnabled) throw "この操作は -debug を付けて起動した場合だけ使えます。";
  // Browser preview is isolated: it never accesses files, accounts or services.
  switch (command) {
    case "preview_theme": return undefined as T;
    case "get_update_status": return {currentVersion:"0.2.1",autoUpdate:previewAutoUpdate,supported:false,canCheck:false,disabledReason:"画面プレビューでは更新の通信・適用を行いません。",phase:"idle",version:null,notes:null,downloaded:0,total:null,checkedAtMs:null,message:null} as T;
    case "set_auto_update": previewAutoUpdate=Boolean(args?.enabled); return undefined as T;
    case "check_updates": throw "画面プレビューでは更新の通信・適用を行いません。";
    case "get_debug_snapshot": return structuredClone(previewDebug) as T;
    case "clear_debug_events":
      previewDebug={startedAtMs:Date.now(),live:emptyDebugCounts(),tests:emptyDebugCounts(),events:[]};
      return undefined as T;
    case "run_debug_receive_test": {
      if(args?.registered && !previewSettings.whitelistChannelIds.length) throw "先に許可リストへ1件以上追加してください。";
      const restricted=previewSettings.receiveMode==="whitelist"||(previewSettings.receiveMode==="auto"&&args?.presence!=="online");
      previewDebugEvent(args?.raw ? "received_message" : "received_notification");
      if((restricted&&!args?.registered)||(!restricted&&args?.raw))previewDebugEvent("filtered_mode");
      else {previewDebugEvent("queued");previewDebugEvent("validated");}
      return structuredClone(previewDebug) as T;
    }
    case "complete_onboarding":
      if (!previewStatus.configured) throw "Discord の接続設定を保存してから完了してください。";
      previewStatus.onboardingPending = false;
      return undefined as T;
    case "preview_reset_onboarding":
      previewStatus.onboardingPending = true;
      previewStatus.configured = false;
      previewStatus.paused = false;
      previewSettings = { ...previewSettings, clientId: "", secretSet: false };
      return undefined as T;
    case "get_status":
      if (previewStatus.debugEnabled) previewStatus.overlay = false;
      previewStatus.whitelistOnly =
        previewStatus.receiveMode === "whitelist" ||
        (previewStatus.receiveMode === "auto" &&
          !["online", "idle"].includes(previewStatus.botStatus));
      return { ...previewStatus } as T;
    case "set_receive_mode": {
      const mode = args?.mode as ReceiveMode;
      if (mode === "auto" && !previewStatus.botConfigured)
        throw "許可リストの設定で Bot の情報を保存してください。";
      previewStatus.receiveMode = mode;
      previewSettings.receiveMode = mode;
      previewStatus.botStatus = mode === "auto" ? "unknown" : "disabled";
      return undefined as T;
    }
    case "preview_set_presence":
      previewStatus.botStatus = args?.status as BotStatus;
      return undefined as T;
    case "set_privacy":
      previewStatus.privacy = Boolean(args?.enabled);
      return undefined as T;
    case "get_settings":
      previewStatus.paused = true;
      return { ...previewSettings, privacyMode: previewStatus.privacy } as T;
    case "save_settings": {
      const input = args?.input as SettingsInput;
      if (
        input.whitelistChannelIds.length > 32 ||
        new Set(input.whitelistChannelIds).size !==
          input.whitelistChannelIds.length ||
        input.whitelistChannelIds.some(
          (id) =>
            !/^[1-9]\d{0,19}$/.test(id) || BigInt(id) > 18446744073709551615n,
        )
      )
        throw {
          field: "whitelistChannelIds",
          message:
            "重複しない数字のチャンネル ID を32件以内で登録してください。",
        };
      if (!/^\d+$/.test(input.clientId))
        throw {
          field: "clientId",
          message: "数字の Client ID を入力してください。",
        };
      if (!input.clientSecret?.trim() && !previewSettings.secretSet)
        throw { field: "clientSecret", message: "自分のアプリの Client Secret を入力してください。" };
      for (const [field, value, min, max] of [
        ["notificationTimeout", input.notificationTimeout, 1, 30],
        ["volumePercent", input.volumePercent, 0, 100],
        ["maxTitleLength", input.maxTitleLength, 10, 200],
        ["maxContentLength", input.maxContentLength, 10, 2000],
      ] as const) {
        if (!Number.isFinite(value) || value < min || value > max)
          throw {
            field,
            message: `${min} ～ ${max} の範囲で入力してください。`,
          };
      }
      const botTokenSet =
        Boolean(input.botToken) ||
        (!input.clearBotToken && previewSettings.botTokenSet);
      if (input.receiveMode === "auto") {
        for (const field of ["botGuildId", "botUserId"] as const) {
          if (!channelIdFromInput(input[field]) || !/^\d+$/.test(input[field]))
            throw { field, message: "数字の ID を入力してください。" };
        }
        if (!botTokenSet)
          throw {
            field: "botToken",
            message: "自動判別用 Bot の Token を入力してください。",
          };
      }
      const {
        clientSecret,
        botToken: _botToken,
        clearBotToken: _clearBotToken,
        ...settings
      } = input;
      previewSettings = {
        ...settings,
        secretSet: Boolean(clientSecret) || previewSettings.secretSet,
        botTokenSet,
      };
      previewStatus.privacy = input.privacyMode;
      previewStatus.theme = input.theme;
      previewStatus.paused = false;
      previewStatus.configured = true;
      previewStatus.receiveMode = input.receiveMode;
      previewStatus.whitelistCount = input.whitelistChannelIds.length;
      previewStatus.whitelistStatus = input.whitelistChannelIds.length ? 2 : 0;
      previewStatus.botConfigured =
        botTokenSet && Boolean(input.botGuildId) && Boolean(input.botUserId);
      previewStatus.botStatus =
        input.receiveMode === "auto" ? "unknown" : "disabled";
      return { ...previewSettings } as T;
    }
    case "cancel_settings":
      previewStatus.paused = false;
      return undefined as T;
    default:
      throw "プレビューではこの操作を実行しません。デスクトップアプリで使用できます。";
  }
}
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error)
    return String(error.message);
  return "操作を完了できませんでした。もう一度お試しください。";
}
