# RPC を維持した一般配布の設計レビュー

更新日: 2026-10-08。元コード: `887651e8583583d15164479960f76f27e2a677e4`。

## 採用する方針

- Windows 向け Discord → XSOverlay 通知転送。RPC を維持する。
- 利用者は自分の Discord アカウントで Developer Portal のアプリを作成し、自分の Client ID・Client Secret を使う。
- 初回は案内付きの設定画面を表示する。共通の Secret を配布せず、認証サーバーも導入しない。
- 使いやすさと安全性を優先し、通常表示と配信用表示を切り替えられるようにする。
- 取り込み中の受信制限は手動切り替えと任意の Bot による自動判別の両方に対応する。Bot は自分のステータス判定だけに使い、会話は RPC で受信する。

所有者本人のアカウント・Client ID で、未承認でも動いているという利用者の報告がある。この実績を前提とする。共通 Client ID を不特定多数へ配る方式は今回の配布設計には採用しない。

## Discord の承認条件

公式 OAuth2 資料は `rpc` と `rpc.notifications.read` を承認されたパートナー向けと記載している。RPC 資料には未承認アプリのテスター枠が最大50人とある。許可リストで追加する `messages.read` も公式資料では特定のパートナー向けの scope で、Bot の認証や Privileged Intent とは別の仕組みである。

本人所有のアプリで動くという観測と、公式資料の権限制限は分けて扱う。利用者ごとにアプリを作る方式について、将来の動作や承認不要を公式に保証する資料は確認できていない。現在の受付状況・審査基準・所要日数も、確認した公開資料からは確定できなかった。

この不確実性は保存・通信・設定画面の改善を止める理由にはしない。配布前に別の利用者が自分で作ったアプリでも、初回認証・購読・通知受信・再起動が通ることを確認する。必要なら Developer Support に本人所有アプリでの利用条件を問い合わせる。

Public Client / PKCE は Social SDK の公式資料に記載があるが、今回の RPC 通知 scope への適用は未確認。現方式から Secret を削除するだけでは実装できないため、今回は採用しない。

## 今回の変更

取り込み中には RPC の通常通知 `NOTIFICATION_CREATE` が抑制されるため、許可リストのチャンネルだけ `MESSAGE_CREATE` を購読する。通常時は直接受信イベントを転送せず、Discord の通知設定に従う。DM もチャンネル ID で登録し、通知イベントとの重複を除去する。

本人の取り込み中状態を得るための信頼できる公開 RPC API を確認できなかったため、任意の自分の Bot を使う。Bot と本人が参加するサーバーで Presence Intent を有効にし、GUILDS + GUILD_PRESENCES（257）を使う。対象ユーザーの取得は Request Guild Members の user_ids + presences を使い、全メンバー一覧の Intent は要求しない。Bot 招待の権限は0。Presence Intent は特権 Intent のため、規模によって承認が必要な場合は Portal の案内に従う。

初回状態・復帰時のスナップショットに加えてライブ更新を処理し、古いスナップショットが新しい更新を戻さないようにする。Heartbeat ACK が途絶えると切断し、Bot の状態を不明に戻す。状態不明・オフライン・RPC アカウント不一致は許可リストに制限する。本人の Bot による判定であり、RPC scope の承認要件を変更するものではない。

| 対象 | 実装 | 目的 |
| --- | --- | --- |
| 初回導入 | Tauri 2 + React の5段階チュートリアル。準備、Portal でのアプリ作成と入力、表示選択、接続確認。通常設定は4タブ | JSON 編集を初回の必須作業にしない |
| 再設定 | `--setup`、認証情報変更時に古いトークンを消去、未知の設定を保持 | 別アプリの認証情報を混在させない |
| 秘密の保存 | Windows DPAPI。入力画面の保存時点で暗号化 | 入力した Secret を平文のまま残さない |
| 設定保存 | 一時ファイルから原子的に置換。不正な JSON を初期設定で上書きしない。モード保存とトークン更新を直列化 | 設定破損・意図しない初期化や保存の競合を防ぐ |
| 配信用表示 | ホーム・トレイ、`--privacy` / `--normal`、設定。最後の選択を保存。名前・本文・アイコンを非表示 | 配信での会話の露出を減らす |
| ログ | 本文は標準で非表示。トークン応答は出力しない | コンソール共有時の漏えいを減らす |
| IPC | 部分受信を接続バッファで保持、フレーム長制限、PING/PONG・CLOSE・書込タイムアウト | 受信キャンセル後のフレーム破損を防ぐ |
| RPC | nonce とコマンドを照合し、途中の通知を別キューへ保留 | 別の応答の混同や保留通知の再読込を防ぐ |
| 権限・待ち時間 | 通常は rpc + rpc.notifications.read、許可リスト登録時だけ messages.read を追加。GET_CHANNEL/GET_GUILD は使わない | 必要な場合だけ追加権限を要求する |
| 取り込み中 | 許可リストの各 channel_id に MESSAGE_CREATE を購読。手動または Bot の状態で転送可否を切り替える | 通常通知が抑制されても登録先を受信する |
| Bot 判定 | GUILDS + GUILD_PRESENCES、権限0、対象サーバー・自分のユーザーだけ。状態不明は許可リストに制限 | メッセージ取得権限を Bot に与えず自動化する |
| 認証失敗 | 通信失敗・429・5xx は再試行。`invalid_grant` と設定・権限エラーを区別。承認要求は起動ごとに一度 | 通信障害や拒否で承認画面を繰り返さない |
| 画像 | Discord HTTPS CDN のみ、リダイレクトなし、1秒・256 KiB・キャッシュ上限 | 受信の停滞と過剰な取得を防ぐ |
| 通知待ち | 最大64件・15秒、切断時は破棄 | 古い会話の再表示とメモリ増加を防ぐ |
| 二重起動 | Windows セッション単位の単一起動 | 重複通知と保存の競合を減らす |
| 操作画面 | 接続カード・表示モードの二択・ダミーのプレビュー・テスト・設定・終了。Windows の外観に追従。×でトレイへ収納。設定の入力保持・エラー案内・通知音のファイル参照 | 普段の操作と設定でコンソール・JSON 編集を不要にする |
| 検証 | 通信・保存・認証・匿名化の回帰テスト、Windows CI | 再発を検出する |
| 受信診断 | -debug / --debug では XSOverlay を使わず実受信件数・処理履歴・画面内チェック、独立したダミー受信を確認。本文・ID・秘密を記録しない | Discord からの受信と判定を VR なしで確認する |

配信用表示は、すでに送信した通知や送信が始まった通知、Discord 本体の画面を消去しない。配信前に配信用モードで起動する運用が必要。

旧トークンの追加権限は、要求 scope を減らしても自動では取り消されない。最小権限で承認し直す場合は Discord の承認済みアプリから解除してから、このツールを再起動する。

## 次の改善候補

1. 一時ミュートと通知音の試聴。
2. VR 中のショートカットを設定可能にする。現在はホーム・トレイで切り替える。
3. 共有用の接続診断を追加する。Secret・トークン・本文を診断ファイルへ含めない。状態表示とテスト通知は GUI に実装済み。
4. 実装済みのチャンネル・DM 許可リストに、連続通知のまとめ表示を追加する。
5. 署名付き配布・リリース手順を整える。

## 配布前の確認

- 別の利用者が、自分のアカウントと新規作成したアプリで導入できること。
- 認証の拒否、権限撤回、無効な Secret、ネット切断、429・5xx。
- Discord / XSOverlay の再起動、二重起動、送信待ち中の配信用切替。
- 初回設定画面のキーボード操作、日本語表示、画面拡大率、保存失敗。
- messages.read の追加承認、DM・サーバーチャンネルの直接購読、取り込み中の実機受信。
- Presence Intent、権限0の Bot 招待、オンライン・退席・取り込み中・Invisible・切断・RPC アカウント不一致。
- ZIP に実際の設定・トークン・コンパイラー・テスト生成物を含めないこと。

ローカル検証の結果は [IMPROVEMENTS.md](../IMPROVEMENTS.md) に記録する。回帰テストは Discord / XSOverlay の実機確認を代替しない。

## Developer Support 問い合わせ草案（未送信）

送信先候補: https://support-dev.discord.com/hc/en-us/requests/new

> We are developing an open-source Windows utility that forwards notifications from the local Discord desktop client to the local XSOverlay VR application. Each user will create their own Discord application and use their own application ID and secret. We do not distribute a shared secret or use a shared authentication server. Notification processing stays on the user's computer.
>
> We currently have successful local IPC operation with the application owner's account and their own unapproved application. Could you clarify whether this owner-only use of `rpc` and `rpc.notifications.read` is supported for a publicly distributed desktop utility whose users each create their own application? Is additional approval required for this model?
>
> We also plan to subscribe to `MESSAGE_CREATE` only for channels or DM conversations explicitly registered by the user, requiring `messages.read`, to support allowlisted notifications while Do Not Disturb suppresses normal notification events. Does this additional scope require a separate approval process for the same owner-only model?
>
> If approval is required, what is the current process and eligibility for this non-game utility? Please let us know what supporting material, privacy documentation, or security information is required.

必要な App ID・連絡先は所有者が入力する。Secret・トークンは添付しない。この草案は送信していない。

## 一次資料

- [Discord: RPC](https://github.com/discord/discord-api-docs/blob/main/developers/topics/rpc.mdx)
- [Discord: OAuth2](https://github.com/discord/discord-api-docs/blob/main/developers/topics/oauth2.mdx)
- [Discord: RPC error codes](https://github.com/discord/discord-api-docs/blob/main/developers/topics/opcodes-and-status-codes.mdx)
- [Discord Social SDK: Authentication](https://discord.com/developers/docs/social-sdk/authentication.html)
- [Discord Developer Support](https://support-dev.discord.com/hc/en-us/requests/new)
- [Discord Gateway / Privileged Intents](https://docs.discord.com/developers/events/gateway)
- [Discord Gateway Events / Request Guild Members](https://docs.discord.com/developers/events/gateway-events)
