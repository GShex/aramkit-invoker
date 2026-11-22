use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use tungstenite::{Message, WebSocket};

use super::is_allowed_champion_url;

pub(super) fn navigate(profile: &Path, target_id: &mut Option<String>, url: &str) -> Result<bool> {
    let endpoint = std::fs::read_to_string(profile.join("DevToolsActivePort"))
        .context("专用浏览器尚未提供页面连接")?;
    let mut lines = endpoint.lines();
    let port: u16 = lines.next().context("缺少浏览器端口")?.parse()?;
    let browser_path = lines.next().context("缺少浏览器连接路径")?;
    let browser_id = browser_path.strip_prefix("/devtools/browser/");
    if !browser_id.is_some_and(|id| {
        !id.is_empty() && id.bytes().all(|ch| ch.is_ascii_hexdigit() || ch == b'-')
    }) {
        bail!("浏览器连接路径无效");
    }

    // Use only the loopback endpoint from this dedicated Edge profile.
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_secs(1);
    let stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(timeout))?;
    let (mut socket, _) = tungstenite::client(format!("ws://{address}{browser_path}"), stream)?;

    let targets = command(
        &mut socket,
        json!({ "id": 1, "method": "Target.getTargets" }),
    )?;
    let pages: Vec<_> = targets["targetInfos"]
        .as_array()
        .context("浏览器未返回页面列表")?
        .iter()
        .filter(|target| target["type"] == "page" && target["openerId"].is_null())
        .collect();
    let Some(first) = pages.first() else {
        *target_id = None;
        return Ok(false);
    };
    let target = pages
        .iter()
        .copied()
        .find(|page| page["targetId"].as_str() == target_id.as_deref())
        .or_else(|| {
            pages
                .iter()
                .copied()
                .find(|page| page["url"].as_str().is_some_and(is_allowed_champion_url))
        })
        .or_else(|| (pages.len() == 1).then_some(*first))
        .context("无法唯一确定原来的 ARAMKit 页面")?;
    let id = target["targetId"].as_str().context("页面标识无效")?;
    *target_id = Some(id.to_owned());
    let session = command(
        &mut socket,
        json!({
            "id": 2,
            "method": "Target.attachToTarget",
            "params": { "targetId": id, "flatten": true },
        }),
    )?;
    let session_id = session["sessionId"].as_str().context("页面连接无效")?;
    command(
        &mut socket,
        json!({ "id": 3, "sessionId": session_id, "method": "Page.stopLoading" }),
    )?;
    let reload = target["url"].as_str() == Some(url);
    let result = command(
        &mut socket,
        json!({
            "id": 4,
            "sessionId": session_id,
            "method": if reload { "Page.reload" } else { "Page.navigate" },
            "params": if reload { json!({}) } else { json!({ "url": url }) },
        }),
    )?;
    if let Some(error) = result["errorText"].as_str() {
        bail!("{error}");
    }
    let _ = socket.close(None);
    Ok(true)
}

fn command(socket: &mut WebSocket<TcpStream>, request: Value) -> Result<Value> {
    socket.send(Message::Text(request.to_string().into()))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Message::Text(text) = socket.read()? {
            let response: Value = serde_json::from_str(text.as_str())?;
            if response["id"] == request["id"] {
                if let Some(error) = response["error"]["message"].as_str() {
                    bail!("{error}");
                }
                return response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| anyhow!("浏览器未返回命令结果"));
            }
        }
    }
    bail!("等待浏览器响应超时");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_the_homepage_target_for_navigation_and_reload() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();
        let profile = tempfile::tempdir().expect("profile");
        std::fs::write(
            profile.path().join("DevToolsActivePort"),
            format!("{port}\n/devtools/browser/12345678-abcd-1234-abcd-123456789abc\n"),
        )
        .expect("endpoint");
        let worker = std::thread::spawn(move || {
            for (current, expected) in [
                (super::super::ARAMKIT_HOME, "Page.navigate"),
                ("https://aramkit.cn/zh-CN/champions/ahri", "Page.reload"),
            ] {
                let (stream, _) = listener.accept().expect("connection");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                let mut socket = tungstenite::accept(stream).expect("websocket");
                for method in [
                    "Target.getTargets",
                    "Target.attachToTarget",
                    "Page.stopLoading",
                    expected,
                ] {
                    let request: Value = serde_json::from_str(
                        socket.read().expect("request").to_text().expect("text"),
                    )
                    .expect("json");
                    assert_eq!(request["method"], method);
                    let result = match method {
                        "Target.getTargets" => json!({
                            "targetInfos": [{ "type": "page", "targetId": "same-page", "url": current }]
                        }),
                        "Target.attachToTarget" => {
                            assert_eq!(request["params"]["targetId"], "same-page");
                            json!({ "sessionId": "session" })
                        }
                        "Page.navigate" => {
                            assert_eq!(
                                request["params"]["url"],
                                "https://aramkit.cn/zh-CN/champions/ahri"
                            );
                            json!({})
                        }
                        _ => json!({}),
                    };
                    socket
                        .send(Message::Text(
                            json!({ "id": request["id"], "result": result })
                                .to_string()
                                .into(),
                        ))
                        .expect("response");
                }
            }
        });
        let mut target = None;
        for _ in 0..2 {
            assert!(navigate(
                profile.path(),
                &mut target,
                "https://aramkit.cn/zh-CN/champions/ahri"
            )
            .expect("navigate"));
            assert_eq!(target.as_deref(), Some("same-page"));
        }
        worker.join().expect("server");
    }
}
