//! «Свой сервер»: the user's VPN subscription (the same link Happ / v2rayN take) inside SC Desk, so no VPN app
//! has to run. It is not a VPN of the computer (no TUN, no adapter): sing-box, the open protocol engine many
//! clients use, runs as a small local proxy while SC Desk is open, and only SoundCloud goes through it
//! (bypass.rs sends SoundCloud's connections there, the browser windows follow the same PAC).
//!
//! sing-box is not in the installer: it is downloaded from its official GitHub release the first time a server
//! is picked (checked by its checksum) into the app data folder. It is tied to SC Desk by a Windows job object:
//! it ends with the app, even if the app is killed.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU16, Ordering},
        Mutex,
    },
};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::{
    error::{AppError, AppResult},
    net::HttpClient,
};

const VERSION: &str = "1.14.3";
const ZIP_URL: &str = "https://github.com/SagerNet/sing-box/releases/download/v1.14.3/sing-box-1.14.3-windows-amd64.zip";
const ZIP_SHA256: &str = "d8b86ef58936dba9230ec6a57ffa4243933f3c14ef4809828b6c553440ed1518";

/// sing-box's local port while it runs (0: not running)
static PORT: AtomicU16 = AtomicU16::new(0);
static CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);

pub fn port() -> u16 {
    PORT.load(Ordering::Relaxed)
}

/// One server of a subscription.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub name: String,
    /// the link as it came (kept to rebuild the outbound)
    pub link: String,
    /// sing-box can use it (xhttp and a few rare transports cannot)
    pub ok: bool,
}

/// What is kept (in the Windows credential store: a subscription link is a key to the servers).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Saved {
    pub url: String,
    pub servers: Vec<Server>,
    /// the picked server, None: not used
    pub selected: Option<usize>,
}

fn entry() -> AppResult<keyring::Entry> {
    let service = match crate::secrets::profile() {
        Some(p) => format!("dev.scdesk.app.{p}.vpn"),
        None => "dev.scdesk.app.vpn".to_owned(),
    };
    keyring::Entry::new(&service, "subscription").map_err(|e| AppError::Keyring(e.to_string()))
}

pub fn load() -> Saved {
    entry().ok().and_then(|e| e.get_password().ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(s: &Saved) -> AppResult<()> {
    entry()?.set_password(&serde_json::to_string(s)?).map_err(|e| AppError::Keyring(e.to_string()))
}

// ------------------------------------------------------------------ subscription

/// The servers of a subscription link (or of one server link pasted as it is).
/// Which VPN app SC Desk introduces itself as to the subscription panel: "auto" tries them in turn.
const CLIENTS: [(&str, &str); 6] = [
    ("happ", "Happ/4.3.0"),
    ("v2rayn", "v2rayN/7.0"),
    ("hiddify", "HiddifyNext/2.5.7"),
    ("nekobox", "NekoBox/1.3.0"),
    ("streisand", "Streisand/1.6"),
    // Clash-family apps (Koala Clash, Clash Verge) get a YAML profile: read too
    ("clash", "clash-verge/v2.2.3"),
];

pub async fn fetch(http: &HttpClient, url: &str, client: &str) -> AppResult<Vec<Server>> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return servers_of(url).ok_or_else(|| AppError::Other("в ссылке не нашлось сервера".into()));
    }
    // Many panels give the real servers only to the clients they allow and a stub ("Установите Incy или
    // Обновите Happ") to anybody else: SC Desk asks as Happ first (with the device headers Happ sends), then
    // as v2rayN (panels answer it with plain links; to Clash they send YAML, which is not read here).
    let hwid = device_id();
    let device = [("x-hwid", hwid.as_str()), ("x-device-os", "Windows"), ("x-ver-os", "10"), ("x-device-model", "PC")];
    let tries: Vec<&str> = match CLIENTS.iter().find(|(k, _)| *k == client) {
        Some((_, ua)) => vec![*ua],
        None => CLIENTS.iter().map(|(_, ua)| *ua).collect(),
    };
    let mut encrypted = false;
    for ua in tries {
        // the device headers too: panels that count devices want them (Happ and others send them)
        let headers = device.to_vec();
        let Ok(body) = http.get_text_ua(url, ua, &headers).await else { continue };
        if body.trim_start().starts_with("happ://crypt") {
            encrypted = true;
            continue;
        }
        if let Some(s) = servers_of(&body).filter(|s| !is_stub(s)) {
            return Ok(s);
        }
    }
    Err(AppError::Other(if encrypted {
        "подписка зашифрована для Happ (happ://crypt…): её может открыть только сам Happ. Попросите у сервиса обычную ссылку на подписку или вставьте ссылку на один сервер (vless://…) из Happ: долгое нажатие на сервер → «Поделиться» / «Копировать»".into()
    } else {
        "сервис отдаёт сервера только своим программам (вместо них пришла заглушка). Вставьте ссылку на один сервер (vless://…): в Happ долгое нажатие на сервер → «Поделиться» / «Копировать»".into()
    }))
}

/// The servers of a subscription body: links, base64 of them, or a Clash / mihomo YAML profile.
fn servers_of(body: &str) -> Option<Vec<Server>> {
    if let Some(s) = clash_servers(body) {
        return Some(s);
    }
    let text = if body.contains("://") { body.to_owned() } else { decode_b64(body.trim()).unwrap_or_default() };
    let servers: Vec<Server> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.contains("://"))
        .map(|l| Server { name: name_of(l), link: l.to_owned(), ok: outbound(l).is_some() })
        .collect();
    (!servers.is_empty()).then_some(servers)
}

/// A stub instead of servers: a few entries that only tell to install or update an app, or point nowhere.
fn is_stub(servers: &[Server]) -> bool {
    let words = ["установите", "обновите", "install", "update", "download", "скачайте"];
    servers.len() <= 3
        && servers.iter().all(|s| {
            let n = s.name.to_lowercase();
            let nowhere = outbound(&s.link).is_some_and(|o| matches!(o["server"].as_str(), Some("0.0.0.0" | "127.0.0.1" | "localhost" | "")));
            words.iter().any(|w| n.contains(w)) || nowhere
        })
}

/// The same id every time on this computer (some panels count devices by it), nothing personal in it.
fn device_id() -> String {
    use sha2::Digest;
    let seed = format!("{}|{}|sc-desk", std::env::var("COMPUTERNAME").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    hex::encode(&sha2::Sha256::digest(seed.as_bytes())[..16])
}

fn decode_b64(s: &str) -> Option<String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != '=').collect();
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    STANDARD_NO_PAD.decode(&clean).or_else(|_| URL_SAFE_NO_PAD.decode(&clean)).ok().and_then(|b| String::from_utf8(b).ok())
}

fn name_of(link: &str) -> String {
    if let Some(rest) = link.strip_prefix("vmess://") {
        if let Some(v) = decode_b64(rest).and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            return v["ps"].as_str().unwrap_or("vmess").to_owned();
        }
    }
    link.rsplit_once('#').map(|(_, n)| urlencoding::decode(n).map(|c| c.into_owned()).unwrap_or_else(|_| n.to_owned())).unwrap_or_else(|| link.split("://").next().unwrap_or("").to_owned())
}

/// A Clash profile ("proxies:" list) as servers; each is kept as its sing-box outbound ("sb:" + JSON).
fn clash_servers(body: &str) -> Option<Vec<Server>> {
    if !body.contains("proxies:") {
        return None;
    }
    let doc: serde_yaml::Value = serde_yaml::from_str(body).ok()?;
    let list = doc.get("proxies")?.as_sequence()?;
    let servers: Vec<Server> = list
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_owned();
            let out = clash_outbound(p);
            let link = out.as_ref().map_or_else(String::new, |o| format!("sb:{o}"));
            Some(Server { name, ok: out.is_some(), link })
        })
        .collect();
    (!servers.is_empty()).then_some(servers)
}

/// One proxy of a Clash profile as a sing-box outbound; None: a kind or transport sing-box lacks (xhttp, the
/// post-quantum VLESS "encryption", …).
fn clash_outbound(p: &serde_yaml::Value) -> Option<Value> {
    let s = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let server = s("server");
    let port = p.get("port")?.as_u64()? as u16;
    let alpn: Vec<String> = p.get("alpn").and_then(|a| a.as_sequence()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let fp = match s("client-fingerprint").as_str() {
        "" => "chrome".to_owned(),
        f => f.to_owned(),
    };
    let tls = |sni: String| {
        let mut t = json!({ "enabled": true, "server_name": if sni.is_empty() { server.clone() } else { sni }, "utls": { "enabled": true, "fingerprint": fp } });
        if !alpn.is_empty() {
            t["alpn"] = json!(alpn);
        }
        if let Some(r) = p.get("reality-opts") {
            t["reality"] = json!({ "enabled": true, "public_key": r.get("public-key").and_then(|v| v.as_str()).unwrap_or(""),
                "short_id": r.get("short-id").and_then(|v| v.as_str()).unwrap_or("") });
        }
        t
    };
    let mut o = match s("type").as_str() {
        "vless" => {
            let enc = s("encryption");
            if !enc.is_empty() && enc != "none" {
                return None;
            }
            let mut o = json!({ "type": "vless", "server": server, "server_port": port, "uuid": s("uuid") });
            if !s("flow").is_empty() {
                o["flow"] = json!(s("flow"));
            }
            if p.get("tls").and_then(|v| v.as_bool()).unwrap_or(false) {
                o["tls"] = tls(s("servername"));
            }
            o
        }
        "vmess" => {
            let mut o = json!({ "type": "vmess", "server": server, "server_port": port, "uuid": s("uuid"),
                "security": if s("cipher").is_empty() { "auto".to_owned() } else { s("cipher") }, "alter_id": p.get("alterId").and_then(|v| v.as_u64()).unwrap_or(0) });
            if p.get("tls").and_then(|v| v.as_bool()).unwrap_or(false) {
                o["tls"] = tls(s("servername"));
            }
            o
        }
        "trojan" => json!({ "type": "trojan", "server": server, "server_port": port, "password": s("password"), "tls": tls(s("sni")) }),
        "hysteria2" | "hy2" => {
            let mut o = json!({ "type": "hysteria2", "server": server, "server_port": port, "password": s("password"),
                "tls": { "enabled": true, "server_name": if s("sni").is_empty() { server.clone() } else { s("sni") }, "alpn": if alpn.is_empty() { vec!["h3".to_owned()] } else { alpn.clone() },
                         "insecure": p.get("skip-cert-verify").and_then(|v| v.as_bool()).unwrap_or(false) } });
            if s("obfs") == "salamander" {
                o["obfs"] = json!({ "type": "salamander", "password": s("obfs-password") });
            }
            return Some(o);
        }
        "ss" | "shadowsocks" => return Some(json!({ "type": "shadowsocks", "server": server, "server_port": port, "method": s("cipher"), "password": s("password") })),
        _ => return None,
    };
    match s("network").as_str() {
        "" | "tcp" => {}
        "ws" => {
            let ws = p.get("ws-opts");
            let path = ws.and_then(|w| w.get("path")).and_then(|v| v.as_str()).unwrap_or("/");
            let host = ws.and_then(|w| w.get("headers")).and_then(|h| h.get("Host")).and_then(|v| v.as_str()).unwrap_or("");
            o["transport"] = json!({ "type": "ws", "path": path, "headers": if host.is_empty() { json!({}) } else { json!({ "Host": host }) } });
        }
        "grpc" => {
            let svc = p.get("grpc-opts").and_then(|g| g.get("grpc-service-name")).and_then(|v| v.as_str()).unwrap_or("");
            o["transport"] = json!({ "type": "grpc", "service_name": svc });
        }
        _ => return None,
    }
    Some(o)
}

/// The link as a sing-box outbound; None: a kind sing-box does not speak.
pub fn outbound(link: &str) -> Option<Value> {
    // a server of a Clash profile, already in sing-box's terms
    if let Some(json) = link.strip_prefix("sb:") {
        return serde_json::from_str(json).ok();
    }
    if let Some(rest) = link.strip_prefix("vmess://") {
        let v: Value = serde_json::from_str(&decode_b64(rest)?).ok()?;
        let s = |k: &str| v[k].as_str().map(str::to_owned).or_else(|| v[k].as_u64().map(|n| n.to_string())).unwrap_or_default();
        let mut o = json!({ "type": "vmess", "server": s("add"), "server_port": s("port").parse::<u16>().ok()?, "uuid": s("id"),
            "security": if s("scy").is_empty() { "auto".into() } else { s("scy") }, "alter_id": s("aid").parse::<u32>().unwrap_or(0) });
        transport(&mut o, &s("net"), &s("path"), &s("host"), &s("path"))?;
        if s("tls") == "tls" {
            o["tls"] = json!({ "enabled": true, "server_name": if s("sni").is_empty() { s("host") } else { s("sni") }, "utls": { "enabled": true, "fingerprint": "chrome" } });
        }
        return Some(o);
    }
    let u = Url::parse(link).ok()?;
    let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap_or_default();
    let host = u.host_str()?.trim_matches(['[', ']']).to_owned();
    let port = u.port()?;
    let user = urlencoding::decode(u.username()).ok()?.into_owned();
    let mut o = match u.scheme() {
        "vless" => {
            let mut o = json!({ "type": "vless", "server": host, "server_port": port, "uuid": user });
            if !q("flow").is_empty() {
                o["flow"] = json!(q("flow"));
            }
            o
        }
        "trojan" => json!({ "type": "trojan", "server": host, "server_port": port, "password": user }),
        "ss" => {
            // ss://base64(method:password)@host:port or ss://method:password@host:port
            let creds = decode_b64(&user).filter(|c| c.contains(':')).unwrap_or_else(|| format!("{user}:{}", u.password().unwrap_or("")));
            let (method, password) = creds.split_once(':')?;
            return Some(json!({ "type": "shadowsocks", "server": host, "server_port": port, "method": method, "password": urlencoding::decode(password).ok()?.into_owned() }));
        }
        "hysteria2" | "hy2" => {
            let mut o = json!({ "type": "hysteria2", "server": host, "server_port": port, "password": user,
                "tls": { "enabled": true, "server_name": if q("sni").is_empty() { host.clone() } else { q("sni") }, "insecure": q("insecure") == "1" } });
            if q("obfs") == "salamander" {
                o["obfs"] = json!({ "type": "salamander", "password": q("obfs-password") });
            }
            return Some(o);
        }
        _ => return None,
    };
    transport(&mut o, &q("type"), &q("path"), &q("host"), &q("serviceName"))?;
    match q("security").as_str() {
        "tls" | "reality" => {
            let fp = if q("fp").is_empty() { "chrome".to_owned() } else { q("fp") };
            let mut tls = json!({ "enabled": true, "server_name": if q("sni").is_empty() { host } else { q("sni") }, "utls": { "enabled": true, "fingerprint": fp } });
            if !q("alpn").is_empty() {
                tls["alpn"] = json!(q("alpn").split(',').collect::<Vec<_>>());
            }
            if q("security") == "reality" {
                tls["reality"] = json!({ "enabled": true, "public_key": q("pbk"), "short_id": q("sid") });
            }
            o["tls"] = tls;
        }
        _ => {}
    }
    Some(o)
}

/// ws / grpc / http-upgrade on top; plain tcp needs nothing. None: a transport sing-box lacks (xhttp …).
fn transport(o: &mut Value, kind: &str, path: &str, host: &str, service: &str) -> Option<()> {
    match kind {
        "" | "tcp" | "raw" => {}
        "ws" => o["transport"] = json!({ "type": "ws", "path": if path.is_empty() { "/" } else { path }, "headers": if host.is_empty() { json!({}) } else { json!({ "Host": host }) } }),
        "grpc" => o["transport"] = json!({ "type": "grpc", "service_name": service }),
        "httpupgrade" => o["transport"] = json!({ "type": "httpupgrade", "path": path, "host": host }),
        _ => return None,
    }
    Some(())
}

// ------------------------------------------------------------------ the engine

fn exe(dir: &Path) -> PathBuf {
    dir.join(format!("sing-box-{VERSION}.exe"))
}

/// sing-box, downloaded and checked the first time.
async fn engine(http: &HttpClient, dir: &Path) -> AppResult<PathBuf> {
    let path = exe(dir);
    if path.exists() {
        return Ok(path);
    }
    tokio::fs::create_dir_all(dir).await?;
    let zip = dir.join("sing-box.zip.part");
    tracing::info!("vpn: downloading sing-box {VERSION}");
    let sha = http.download_file(ZIP_URL, &zip).await?;
    if sha != ZIP_SHA256 {
        let _ = tokio::fs::remove_file(&zip).await;
        return Err(AppError::Other(format!("sing-box: контрольная сумма не совпала ({sha})")));
    }
    let (zip2, out) = (zip.clone(), path.clone());
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        let mut a = zip::ZipArchive::new(std::fs::File::open(&zip2)?).map_err(|e| AppError::Other(e.to_string()))?;
        let name = a.file_names().find(|n| n.ends_with("sing-box.exe")).ok_or_else(|| AppError::Other("sing-box.exe нет в архиве".into()))?.to_owned();
        let mut f = a.by_name(&name).map_err(|e| AppError::Other(e.to_string()))?;
        let tmp = out.with_extension("exe.part");
        std::io::copy(&mut f, &mut std::fs::File::create(&tmp)?)?;
        std::fs::rename(&tmp, &out)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))??;
    let _ = tokio::fs::remove_file(&zip).await;
    Ok(path)
}

/// Stops sing-box (no server picked, or the app is closing).
pub fn stop() {
    PORT.store(0, Ordering::Relaxed);
    if let Some(mut c) = CHILD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Runs sing-box with this server as a local SOCKS proxy (the previous one is stopped).
pub async fn start(http: &HttpClient, dir: &Path, link: &str) -> AppResult<()> {
    let out = outbound(link).ok_or_else(|| AppError::Other("этот сервер sing-box не поддерживает".into()))?;
    start_outbound(http, dir, out).await
}

async fn start_outbound(http: &HttpClient, dir: &Path, out: Value) -> AppResult<()> {
    stop();
    let exe = engine(http, dir).await?;
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
    let config = json!({
        "log": { "level": "warn" },
        // the server's own name and the sites are looked up over HTTPS (DoH): a provider's DNS may lie or be cut
        "dns": { "servers": [{ "type": "https", "server": "1.1.1.1", "tag": "local" }] },
        "route": { "default_domain_resolver": "local" },
        "inbounds": [{ "type": "socks", "listen": "127.0.0.1", "listen_port": port }],
        "outbounds": [out],
    });
    let cfg = dir.join("sing-box.json");
    tokio::fs::write(&cfg, serde_json::to_vec(&config)?).await?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(["run", "-c"]).arg(&cfg).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let child = cmd.spawn()?;
    tie_to_app(&child);
    *CHILD.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    // ready when it listens
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            PORT.store(port, Ordering::Relaxed);
            tracing::info!(port, "vpn: sing-box is up");
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    stop();
    Err(AppError::Other("sing-box не запустился".into()))
}

/// sing-box ends with SC Desk whatever way SC Desk ends (a job object that kills its processes when closed).
#[cfg(windows)]
fn tie_to_app(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    static JOB: std::sync::OnceLock<isize> = std::sync::OnceLock::new();
    // SAFETY: a job object is created once and never closed (closing it is what kills sing-box, at exit);
    // the info struct is a zeroed local of the right size; the child handle is valid while `child` lives.
    unsafe {
        let job = *JOB.get_or_init(|| {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
            job as isize
        });
        AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
    }
}

#[cfg(not(windows))]
fn tie_to_app(_child: &std::process::Child) {}

#[cfg(test)]
mod tests {
    use super::outbound;

    /// sing-box downloaded and run, SoundCloud through it (a "direct" outbound stands in for a server).
    /// `cargo test vpn_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn vpn_live() {
        let mut cfg = crate::config::AppConfig::default();
        cfg.ui = serde_json::json!({ "scBypass": "off" });
        let http = crate::net::HttpClient::new(&cfg).unwrap();
        let dir = std::env::temp_dir().join("scdesk-vpn-test");
        super::start_outbound(&http, &dir, serde_json::json!({ "type": "direct" })).await.unwrap();
        assert!(super::port() != 0);
        let r = http.get("https://soundcloud.com/", crate::net::Profile::Document, None).await;
        println!("through sing-box: {:?}", r.map(|r| r.status()));
        super::stop();
    }

    #[test]
    fn a_clash_profile_gives_its_servers() {
        let yaml = "mixed-port: 7890\nproxies:\n- name: NL\n  type: vless\n  server: 1.2.3.4\n  port: 8443\n  uuid: 11111111-2222-3333-4444-555555555555\n  flow: xtls-rprx-vision\n  tls: true\n  servername: a.example\n  client-fingerprint: firefox\n  reality-opts:\n    public-key: KEY\n    short-id: ab\n- name: WS\n  type: vless\n  server: w.example\n  port: 443\n  uuid: u\n  tls: true\n  servername: w.example\n  network: ws\n  ws-opts:\n    path: /ws\n    headers:\n      Host: w.example\n- name: TR\n  type: trojan\n  server: t.example\n  port: 7443\n  password: p\n  sni: t.example\n- name: HY\n  type: hysteria2\n  server: h.example\n  port: 8443\n  password: p\n  sni: h.example\n- name: XH\n  type: vless\n  server: x.example\n  port: 443\n  uuid: u\n  network: xhttp\n- name: PQ\n  type: vless\n  server: q.example\n  port: 443\n  uuid: u\n  encryption: mlkem768x25519plus.native.0rtt.X\nproxy-groups: []\n";
        let s = super::servers_of(yaml).unwrap();
        assert_eq!(s.len(), 6);
        assert_eq!(s.iter().filter(|x| x.ok).count(), 4, "xhttp and the post-quantum encryption are not for sing-box");
        let nl = super::outbound(&s[0].link).unwrap();
        assert_eq!(nl["tls"]["reality"]["public_key"], "KEY");
        assert_eq!(super::outbound(&s[1].link).unwrap()["transport"]["headers"]["Host"], "w.example");
        assert_eq!(super::outbound(&s[3].link).unwrap()["type"], "hysteria2");
    }

    #[test]
    fn a_stub_is_told_from_servers() {
        let stub = vec![super::Server { name: "Установите Incy или Обновите Happ".into(), link: "vless://u@0.0.0.0:1#x".into(), ok: true }];
        assert!(super::is_stub(&stub));
        let real = vec![super::Server { name: "NL 1".into(), link: "vless://u@1.2.3.4:443?security=tls#NL".into(), ok: true }];
        assert!(!super::is_stub(&real));
    }

    #[test]
    fn links() {
        let o = outbound("vless://11111111-2222-3333-4444-555555555555@1.2.3.4:443?type=tcp&security=reality&sni=www.microsoft.com&fp=chrome&pbk=KEY&sid=ab&flow=xtls-rprx-vision#NL%201").unwrap();
        assert_eq!(o["type"], "vless");
        assert_eq!(o["tls"]["reality"]["public_key"], "KEY");
        assert_eq!(o["flow"], "xtls-rprx-vision");
        assert_eq!(super::name_of("vless://u@h:1#NL%201"), "NL 1");
        let t = outbound("trojan://pass@host.com:443?security=tls&type=ws&path=%2Fws&sni=host.com#t").unwrap();
        assert_eq!(t["transport"]["type"], "ws");
        let ss = outbound("ss://YWVzLTEyOC1nY206cGFzcw@5.6.7.8:8388#ss").unwrap();
        assert_eq!(ss["method"], "aes-128-gcm");
        assert_eq!(ss["password"], "pass");
        assert!(outbound("vless://u@h:443?type=xhttp&security=tls").is_none());
        let hy = outbound("hysteria2://secret@9.9.9.9:443?sni=a.com&obfs=salamander&obfs-password=x#h").unwrap();
        assert_eq!(hy["obfs"]["type"], "salamander");
    }
}
