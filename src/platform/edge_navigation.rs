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
    stream.set_read_timeout(Some(timeout))?;
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
    let reload = target["url"].as_str() == Some(url);
    let result = command(
        &mut socket,
        json!({
            "id": 3,
            "sessionId": session_id,
            "method": if reload { "Page.reload" } else { "Page.navigate" },
            "params": if reload { json!({}) } else { json!({ "url": url }) },
        }),
    )?;
    if let Some(error) = result["errorText"].as_str() {
        bail!("{error}");
    }
    Ok(true)
}

fn command(socket: &mut WebSocket<TcpStream>, request: Value) -> Result<Value> {
    socket.send(Message::Text(request.to_string().into()))?;
    let deadline = Instant::now() + Duration::from_secs(2);
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
