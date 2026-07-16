# discord_To_VR

**Discord の通知などを VR 空間で見えるようにする**ためのツールです！

VRゲーム中にDiscordでダイレクトメッセージ (DM) やサーバーでのメンション、画像やリンク等が届いた際、わざわざヘッドセットを外したり画面を切り替えたりすることなく、VR オーバーレイアプリ **[XSOverlay](https://store.steampowered.com/app/1173510/XSOverlay/)** のポップアップ通知として VR 内でそのまま確認できます。

Rust で開発されているため非常に軽く、VRゲームのフレームレートやPCの動作に負担をかけません。また、SteamVR (または XSOverlay) が終了すると自動的にこのアプリも停止します。

---

## 🚀 使い方

### 1. Discord アプリ情報の準備
1. [Discord Developer Portal](https://discord.com/developers/applications) でアプリケーションを新規作成します。
2. 左メニューの **「OAuth2」** を開き、**Redirects** に `http://localhost/` を追加して保存します。
3. **Client ID** と **Client Secret** をコピーします。

### 2. 設定の入力
初回に `discord_To_VR.exe` を一度起動（または同じフォルダに `config.json` を作成）すると、設定ファイル `config.json` が自動生成されます。
メモ帳等で `config.json` を開き、コピーした情報を貼り付けて保存してください。

```json
{
  "CLIENT_ID": "ここに取得した Client ID を入力",
  "CLIENT_SECRET": "ここに取得した Client Secret を入力",
  "REDIRECT_URI": "http://localhost/",
  "NOTIFICATION_SOUND": "",
  "NOTIFICATION_VOLUME": 0.0
}
```

> [!NOTE]
> **通知音について（デフォルト：無音）**
> Discord クライアント本体側で通知音が鳴るため、二重に音が鳴るのを防ぐ目的で **XSOverlay 側からの通知効果音はデフォルトで「無音」** に設定されています。
> もし XSOverlay 側からお好みの通知音やデフォルト音を鳴らしたい場合は、`"NOTIFICATION_SOUND": "default"`（または `.wav` / `.mp3` のファイルパス）にし、`"NOTIFICATION_VOLUME": 0.7` などに設定してください。

### 3. アプリの起動
1. Discord と SteamVR (XSOverlay) が起動した状態で、**`discord_To_VR.exe`** をダブルクリックして起動します。
2. **初回のみ**: Discord の画面に承認確認ポップアップが出ますので **「承認」** をクリックしてください。（次回からは承認不要になります）
3. 接続完了のテスト通知が VR 内に出れば準備完了です！以降、Discord にメッセージが届くたびに VR 内でお知らせします。

---

## 🛠️ ソースコードからビルドする場合
開発用・自前のビルドを行いたい場合は、Rust (MSVC環境) がインストールされた環境で以下を実行してください：

```powershell
cargo build --release
```
`target/release/discord_To_VR.exe` に実行ファイルが生成されます。

---

## 注意
すべてAIに投げて作成したので、所々おかしいところや知識ある方にとってここはどうなのかと思う部分があるかもしれないです
浅い程度でのプログラミング知識でしいていうなれば環境構築とgithubのプロジェクトなどを自分で作ったくらいですw()
ですのでダウンロードや実行は自己責任でお願いします（ちなみにここの注意以外の説明もすべてAIに頼んでみたよ＾＾）
一応ある程度規約には触れないような仕様ではあると思います
もし気になる点があれば個人で改造して使うなり配布しても構わないです
あとそもそもの仕様で取り込み中だと通知が取得できないのでdiscordの赤ステで通知が来ないのは仕様ですので把握のほうお願いします