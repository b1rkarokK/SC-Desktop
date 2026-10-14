//! Getting through a block of SoundCloud by the provider, with no server of anyone's in between.
//!
//! Providers usually block by the site name, which a TLS connection sends in the clear in its very first
//! message (ClientHello, the SNI field). Here that message is cut into two TLS records, the cut falling inside
//! the name, and each record goes in its own TCP packet: the provider's filter does not see the whole name in
//! one piece, SoundCloud's server puts the records back together as TLS allows. The traffic itself is not
//! touched and goes straight to SoundCloud. It does not help against a block by IP address.
//!
//! How it fits with zapret and a VPN: «Авто» (the default) changes nothing while SoundCloud opens as it is
//! (through a VPN, through zapret, or not blocked at all); only after a connection to SoundCloud breaks off is
//! the cutting turned on, for this run of the app. With a proxy set in Settings it is never used.
//!
//! Inside: a tiny HTTPS proxy on 127.0.0.1 (CONNECT only, SoundCloud hosts only) that the HTTP client is sent
//! through for SoundCloud's hosts; it cuts the first message (while the bypass is on) and then just copies
//! bytes both ways. The app's browser windows (sign-in, the check window, the widget) reach SoundCloud through
//! it too: a PAC script sends only SoundCloud's https addresses there, everything else goes direct.

use std::sync::{
    atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering},
    OnceLock,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const OFF: u8 = 0;
const AUTO: u8 = 1;
const ON: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(AUTO);
/// «Авто» saw SoundCloud break off and turned the cutting on
static TRIPPED: AtomicBool = AtomicBool::new(false);
static PORT: AtomicU16 = AtomicU16::new(0);
/// connections cut so far (the log and the live test)
static SERVED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static STARTED: OnceLock<()> = OnceLock::new();

/// SoundCloud's own hosts: the site, the API, pictures and the audio.
pub fn is_soundcloud(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    ["soundcloud.com", "sndcdn.com", "soundcloud.cloud"].iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

/// The setting from config.json → ui.scBypass: "auto" (default) | "on" | "off".
pub fn set_mode_from_ui(ui: &serde_json::Value) {
    let m = match ui["scBypass"].as_str() {
        Some("on") => ON,
        Some("off") => OFF,
        _ => AUTO,
    };
    MODE.store(m, Ordering::Relaxed);
    if m == ON {
        start();
    }
}

/// The cutting is on now (set so, or turned on by «Авто»).
pub fn is_on() -> bool {
    active() && super::vpn::port() == 0
}

fn active() -> bool {
    match MODE.load(Ordering::Relaxed) {
        ON => true,
        AUTO => TRIPPED.load(Ordering::Relaxed),
        _ => false,
    }
}

/// SoundCloud's address is asked over HTTPS (doh.rs) while the bypass is on: a blocking provider's DNS lies too.
pub fn doh_wanted(host: &str) -> bool {
    active() && is_soundcloud(host)
}

/// The local proxy for this URL, if its connection is to be cut.
pub fn proxy_for(url: &url::Url) -> Option<String> {
    let port = PORT.load(Ordering::Relaxed);
    // «Свой сервер» (vpn.rs) running: SoundCloud goes there through this proxy, whatever the bypass says
    let via = active() || super::vpn::port() != 0;
    (port != 0 && via && url.scheme() == "https" && url.host_str().is_some_and(is_soundcloud)).then(|| format!("http://127.0.0.1:{port}"))
}

/// A connection to SoundCloud broke off: in «Авто» the cutting is turned on (once). True: ask again at once.
pub fn on_failure(url: &str) -> bool {
    let sc = url::Url::parse(url).ok().and_then(|u| u.host_str().map(is_soundcloud)).unwrap_or(false);
    if !sc || MODE.load(Ordering::Relaxed) != AUTO || PORT.load(Ordering::Relaxed) == 0 || TRIPPED.swap(true, Ordering::Relaxed) {
        return false;
    }
    tracing::warn!("SoundCloud does not answer directly: cutting the TLS greeting from now on (bypass, auto)");
    true
}

/// The built-in browser's arguments for every window of the app. WebView2 wants the same ones for all the windows
/// of a run, so they are fixed at the first window: a change of the setting reaches the windows after a restart.
/// Tauri's own defaults are kept; with the bypass off, or with a proxy in Settings, they are all there is.
pub fn browser_args() -> &'static str {
    static ARGS: OnceLock<String> = OnceLock::new();
    ARGS.get_or_init(|| {
        // the built-in browser keeps at most 8 MB of web cache (left alone it grew to hundreds of MB) and does not
        // fetch Edge's own components (ad filter lists and the like, 20 MB the app never uses)
        let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disk-cache-size=8388608 --disable-component-update");
        let port = PORT.load(Ordering::Relaxed);
        // always while the local proxy is there: «Свой сервер» may be picked later in this run
        if port != 0 {
            use base64::Engine as _;
            let pac = format!(concat!(
                "function FindProxyForURL(url, host) {{ host = host.toLowerCase(); ",
                "if (url.substring(0, 6) == 'https:' && (host == 'soundcloud.com' || dnsDomainIs(host, '.soundcloud.com') ",
                "|| dnsDomainIs(host, '.sndcdn.com') || dnsDomainIs(host, '.soundcloud.cloud'))) ",
                "return 'PROXY 127.0.0.1:{port}; DIRECT'; return 'DIRECT'; }}"
            ), port = port);
            let b64 = base64::engine::general_purpose::STANDARD.encode(pac);
            args.push_str(&format!(" --proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,{b64}"));
        }
        args
    })
}

/// Starts the local proxy once (a listening socket that costs nothing while idle).
pub fn start() {
    if STARTED.set(()).is_err() {
        return;
    }
    let listener = match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "bypass: no local port");
            return;
        }
    };
    let _ = listener.set_nonblocking(true);
    PORT.store(listener.local_addr().map(|a| a.port()).unwrap_or(0), Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = TcpListener::from_std(listener) else { return };
        loop {
            let Ok((conn, _)) = listener.accept().await else { continue };
            tauri::async_runtime::spawn(async move {
                if let Err(e) = serve(conn).await {
                    tracing::debug!(error = %e, "bypass: connection ended");
                }
            });
        }
    });
}

async fn serve(mut client: TcpStream) -> std::io::Result<()> {
    // "CONNECT host:443 HTTP/1.1 …\r\n\r\n"
    let mut head = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 4096 || client.read(&mut byte).await? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let line = String::from_utf8_lossy(&head);
    let target = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let host = target.rsplit_once(':').map_or(target.as_str(), |(h, _)| h).to_owned();
    // only SoundCloud: nothing else on this computer can use the port to go anywhere
    if !line.starts_with("CONNECT ") || !is_soundcloud(&host) {
        client.write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n").await?;
        return Ok(());
    }
    tracing::debug!(%target, cut = active(), "bypass: connection");
    let port: u16 = target.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap_or(443);
    // «Свой сервер»: on through sing-box (SOCKS5, the name resolved over there), nothing cut
    let vpn = super::vpn::port();
    if vpn != 0 {
        let mut server = TcpStream::connect(("127.0.0.1", vpn)).await?;
        socks_connect(&mut server, &host, port).await?;
        client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
        tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        return Ok(());
    }
    let ips = if active() { super::doh::lookup(&host).await } else { Vec::new() };
    let mut server = match ips.first() {
        Some(ip) => TcpStream::connect((*ip, port)).await?,
        None => TcpStream::connect(&target).await?,
    };
    server.set_nodelay(true)?;
    client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").await?;
    // the greeting, cut inside the site name
    let mut buf = vec![0u8; 16 * 1024];
    let n = client.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    SERVED.fetch_add(1, Ordering::Relaxed);
    // the browser windows always come here for SoundCloud: the greeting is cut only while the bypass is on
    let parts = if active() { cut(&buf[..n], host.as_bytes()) } else { vec![buf[..n].to_vec()] };
    for part in parts {
        server.write_all(&part).await?;
        server.flush().await?;
        // its own packet for each piece
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    tokio::io::copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}

/// SOCKS5 CONNECT to `host:port`, no password (sing-box's local inbound).
async fn socks_connect(s: &mut TcpStream, host: &str, port: u16) -> std::io::Result<()> {
    let bad = || std::io::Error::other("socks refused");
    s.write_all(&[5, 1, 0]).await?;
    let mut r = [0u8; 2];
    s.read_exact(&mut r).await?;
    if r != [5, 0] {
        return Err(bad());
    }
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0 {
        return Err(bad());
    }
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            usize::from(l[0]) + 2
        }
        _ => return Err(bad()),
    };
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest).await?;
    Ok(())
}

/// The first TLS record (a handshake) split in two records, the cut inside `name` when it is found.
/// Anything else is passed as it is.
fn cut(data: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
    if data.len() < 6 || data[0] != 0x16 {
        return vec![data.to_vec()];
    }
    let len = u16::from_be_bytes([data[3], data[4]]) as usize;
    if len < 2 || data.len() < 5 + len {
        return vec![data.to_vec()];
    }
    let body = &data[5..5 + len];
    let at = body.windows(name.len().max(1)).position(|w| w == name).map_or(len / 2, |p| p + (name.len() / 2).max(1));
    let at = at.clamp(1, len - 1);
    let record = |part: &[u8]| {
        let mut r = Vec::with_capacity(5 + part.len());
        r.extend_from_slice(&[0x16, data[1], data[2]]);
        r.extend_from_slice(&(part.len() as u16).to_be_bytes());
        r.extend_from_slice(part);
        r
    };
    let mut out = vec![record(&body[..at]), record(&body[at..])];
    if data.len() > 5 + len {
        out.push(data[5 + len..].to_vec());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{cut, is_soundcloud};

    #[test]
    fn hosts() {
        assert!(is_soundcloud("api-v2.soundcloud.com"));
        assert!(is_soundcloud("cf-media.sndcdn.com"));
        assert!(is_soundcloud("playback.media-streaming.soundcloud.cloud"));
        assert!(!is_soundcloud("notsoundcloud.com"));
        assert!(!is_soundcloud("lrclib.net"));
    }

    #[test]
    fn the_greeting_is_cut_inside_the_name_and_glues_back() {
        let name = b"api-v2.soundcloud.com";
        let mut body = vec![1u8, 0, 0, 40];
        body.extend_from_slice(b"xx");
        body.extend_from_slice(name);
        body.extend_from_slice(b"yyyy");
        let mut data = vec![0x16, 3, 1];
        data.extend_from_slice(&(body.len() as u16).to_be_bytes());
        data.extend_from_slice(&body);
        let parts = cut(&data, name);
        assert_eq!(parts.len(), 2);
        // no piece holds the whole name
        assert!(parts.iter().all(|p| !p.windows(name.len()).any(|w| w == name)));
        // the record bodies put together are the original body
        let glued: Vec<u8> = parts.iter().flat_map(|p| p[5..].to_vec()).collect();
        assert_eq!(glued, body);
        assert!(parts.iter().all(|p| p[0] == 0x16 && u16::from_be_bytes([p[3], p[4]]) as usize == p.len() - 5));
    }

    /// Real network: SoundCloud answers through cut greetings. `cargo test bypass_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn bypass_live() {
        let mut cfg = crate::config::AppConfig::default();
        cfg.ui = serde_json::json!({ "scBypass": "on" });
        let http = crate::net::HttpClient::new(&cfg).unwrap();
        for url in ["https://soundcloud.com/", "https://api-v2.soundcloud.com/", "https://a-v2.sndcdn.com/assets/"] {
            let r = http.get(url, crate::net::Profile::Document, None).await;
            println!("{url}: {:?}", r.as_ref().map(|r| r.status()).map_err(|e| e.to_string()));
        }
        let n = super::SERVED.load(std::sync::atomic::Ordering::Relaxed);
        println!("cut connections: {n}");
        assert!(n > 0);
    }

    /// Edge (the same engine as WebView2) with the app's browser arguments reaches SoundCloud through the proxy.
    /// `cargo test browser_pac_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn browser_pac_live() {
        super::set_mode_from_ui(&serde_json::json!({ "scBypass": "on" }));
        let args = super::browser_args().to_owned();
        let dir = std::env::temp_dir().join("scdesk-pac-test");
        let pac = args.split(' ').find(|a| a.starts_with("--proxy-pac-url=")).unwrap().to_owned();
        let out = tokio::task::spawn_blocking(move || std::process::Command::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
            .args(["--headless=new", "--disable-gpu", &format!("--user-data-dir={}", dir.display()), &pac, "--virtual-time-budget=8000", "--dump-dom", "https://soundcloud.com/"])
            .output());
        let out = tokio::time::timeout(std::time::Duration::from_secs(60), out).await.unwrap().unwrap().unwrap();
        let n = super::SERVED.load(std::sync::atomic::Ordering::Relaxed);
        println!("page bytes: {}, connections through the proxy: {n}", out.stdout.len());
        assert!(n > 0);
    }

    #[test]
    fn other_data_is_left_alone() {
        assert_eq!(cut(b"GET / HTTP/1.1", b"x"), vec![b"GET / HTTP/1.1".to_vec()]);
    }
}
