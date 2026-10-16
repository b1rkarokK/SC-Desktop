//! «Для стрима»: a page for OBS (Browser source) with the song that plays and the current line of its lyrics.
//! A tiny web server on the loopback only (127.0.0.1): nothing outside this computer can reach it. It answers two
//! addresses: `/overlay` (the page, its look is set in the address: ?size=34&color=ffffff&cover=1&next=1&font=…)
//! and `/now` (what plays, as JSON, polled by the page). Off unless the user turns it on (ui.overlay = {on, port}).

use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::{player::Player, state::AppState};

pub const DEFAULT_PORT: u16 = 8765;

static TASK: Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = Mutex::new(None);

/// Starts, restarts or stops the server as the settings say; Err: the port is taken.
pub async fn apply(app: &AppHandle) -> Result<Option<u16>, String> {
    if let Some(old) = TASK.lock().unwrap_or_else(|e| e.into_inner()).take() {
        old.abort();
    }
    let ui = app.state::<AppState>().config.get().ui;
    if !ui["overlay"]["on"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    let port = ui["overlay"]["port"].as_u64().filter(|p| (1024..=65535).contains(p)).map_or(DEFAULT_PORT, |p| p as u16);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| format!("порт {port} занят: {e}"))?;
    let app = app.clone();
    let handle = tauri::async_runtime::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = match tokio::time::timeout(std::time::Duration::from_secs(3), sock.read(&mut buf)).await {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => return,
                };
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
                let route = path.split('?').next().unwrap_or("/");
                let (status, kind, body) = match route {
                    "/overlay" | "/" => ("200 OK", "text/html; charset=utf-8", PAGE.to_owned()),
                    "/now" => ("200 OK", "application/json; charset=utf-8", now(&app).await.to_string()),
                    _ => ("404 Not Found", "text/plain; charset=utf-8", "not found".to_owned()),
                };
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    *TASK.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    tracing::info!(port, "stream overlay is on (127.0.0.1 only)");
    Ok(Some(port))
}

/// What plays now, with the current and the next line of the lyrics (from the cache only: nothing is fetched here).
async fn now(app: &AppHandle) -> serde_json::Value {
    let Some(player) = app.try_state::<Arc<Player>>() else { return json!({ "playing": false }) };
    let s = player.snapshot();
    let Some(t) = s.track.as_ref() else { return json!({ "playing": false }) };
    let state = app.state::<AppState>();
    let nudge = state.config.get().ui["lyricsNudge"][t.id.to_string()].as_i64().unwrap_or(0);
    let pos = player.position_ms() as i64 - nudge;
    let (mut line, mut next) = (String::new(), String::new());
    let mut has_lyrics = false;
    if let Ok(Some((l, _))) = state.db.lyrics_get(t.id).await {
        if l.found && l.synced {
            has_lyrics = true;
            let lines: Vec<&crate::api::lyrics::Line> = l.lines.iter().filter(|x| x.start_ms.is_some()).collect();
            let at = lines.iter().rposition(|x| x.start_ms.unwrap_or(u64::MAX) as i64 <= pos);
            if let Some(i) = at {
                line = lines[i].text.trim().to_owned();
            }
            if let Some(n) = lines.get(at.map_or(0, |i| i + 1)) {
                next = n.text.trim().to_owned();
            }
        }
    }
    json!({
        "playing": s.playing && !s.loading,
        "title": t.title,
        "artist": t.artist,
        "cover": t.artwork_url.as_deref().map(|u| u.replace("-large.", "-t300x300.")),
        "position_ms": pos.max(0),
        "duration_ms": t.duration_ms,
        "has_lyrics": has_lyrics,
        "line": line,
        "next": next,
    })
}

const PAGE: &str = r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><title>SC Desk</title>
<style>
  html,body{margin:0;background:transparent;overflow:hidden;font-family:var(--font,'Segoe UI',Arial,sans-serif)}
  #box{display:flex;align-items:center;gap:16px;padding:12px;color:var(--color,#fff);text-shadow:0 2px 8px rgba(0,0,0,.65);opacity:0;transition:opacity .4s}
  #box.on{opacity:1}
  #cover{width:var(--cover,96px);height:var(--cover,96px);border-radius:12px;object-fit:cover;flex:none;box-shadow:0 4px 18px rgba(0,0,0,.5)}
  #cover.off{display:none}
  #text{min-width:0}
  #title{font-size:var(--size,34px);font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  #artist{font-size:calc(var(--size,34px)*.62);opacity:.8;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
  #line{font-size:calc(var(--size,34px)*.9);font-weight:600;margin-top:.35em;min-height:1.2em}
  #next{font-size:calc(var(--size,34px)*.6);opacity:.55}
</style></head><body>
<div id="box"><img id="cover" alt=""><div id="text"><div id="title"></div><div id="artist"></div><div id="line"></div><div id="next"></div></div></div>
<script>
  const q=new URLSearchParams(location.search), root=document.documentElement.style;
  if(q.get('size'))root.setProperty('--size',q.get('size')+'px');
  if(q.get('color'))root.setProperty('--color','#'+q.get('color').replace('#',''));
  if(q.get('font'))root.setProperty('--font',q.get('font'));
  if(q.get('cover')==='0')document.getElementById('cover').classList.add('off');
  else if(q.get('size'))root.setProperty('--cover',(q.get('size')*2.8)+'px');
  const showNext=q.get('next')!=='0', box=document.getElementById('box');
  const set=(id,t)=>{const e=document.getElementById(id);if(e.textContent!==t)e.textContent=t};
  async function tick(){
    try{
      const d=await (await fetch('/now',{cache:'no-store'})).json();
      if(!d.title||(q.get('hide')==='pause'&&!d.playing)){box.classList.remove('on');return}
      set('title',d.title);set('artist',d.artist);
      set('line',q.get('lyrics')==='0'?'':(d.line||(d.has_lyrics?'♪':'')));
      set('next',showNext&&q.get('lyrics')!=='0'?d.next:'');
      const c=document.getElementById('cover');if(d.cover&&c.getAttribute('src')!==d.cover)c.src=d.cover;
      box.classList.add('on');
    }catch(e){box.classList.remove('on')}
  }
  tick();setInterval(tick,300);
</script></body></html>"#;
