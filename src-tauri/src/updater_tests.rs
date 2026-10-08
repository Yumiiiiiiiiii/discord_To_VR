use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tauri_plugin_updater::UpdaterExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    public_key: String,
    signature: String,
    payload: String,
}

#[tokio::test]
async fn signed_download_rejects_tampering_and_version_replay_and_skips_current_release() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../tests/fixtures/update-signature.json")).unwrap();
    for (version, tampered, expected) in [
        ("0.2.1", false, "valid"),
        ("0.2.1", true, "invalid"),
        ("0.2.2", false, "invalid"),
        ("0.1.0", false, "current"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let manifest=serde_json::to_vec(&json!({"version":version,"platforms":{"windows-x86_64":{"url":format!("http://{address}/fixture"),"signature":fixture.signature}}})).unwrap();
        let mut payload = fixture.payload.as_bytes().to_vec();
        if tampered {
            payload.push(b'!');
        }
        let peer = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let length = stream.read(&mut chunk).await.unwrap();
                    if length == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..length]);
                    assert!(request.len() < 16384);
                }
                let (body, kind) = if request.starts_with(b"GET /manifest ") {
                    (&manifest, "application/json")
                } else {
                    (&payload, "application/octet-stream")
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {kind}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            json!({"pubkey":fixture.public_key,"requireSignedVersion":true}),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        // Local HTTP is confined to this mock runtime; production enforces HTTPS.
        let update = app
            .updater_builder()
            .endpoints(vec![format!("http://{address}/manifest").parse().unwrap()])
            .unwrap()
            .timeout(Duration::from_secs(3))
            .no_proxy()
            .build()
            .unwrap()
            .check()
            .await
            .unwrap();
        if expected == "current" {
            assert!(update.is_none());
        } else {
            let mut downloaded = 0;
            let result = update
                .unwrap()
                .download(|chunk, _| downloaded += chunk, || {})
                .await;
            if expected == "valid" {
                assert_eq!(result.unwrap(), fixture.payload.as_bytes());
                assert_eq!(downloaded, fixture.payload.len());
            } else {
                assert!(result.is_err());
            }
        }
        peer.abort();
        let _ = peer.await;
    }
}
