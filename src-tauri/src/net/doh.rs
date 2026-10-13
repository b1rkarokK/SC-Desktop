//! Addresses of SoundCloud asked over HTTPS (DoH) while the block bypass is on: a provider that blocks
//! SoundCloud often answers DNS with its own stub page address too. The question goes to Cloudflare and Google
//! by their IP addresses (nothing to look up first), the answers are kept for 10 minutes. Everything else, and
//! SoundCloud while the bypass is off, is looked up by the system as usual.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

const SERVERS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];
const KEEP: Duration = Duration::from_secs(600);

static CACHE: Mutex<Option<HashMap<String, (Vec<IpAddr>, Instant)>>> = Mutex::new(None);

fn client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(4)).build().unwrap_or_default())
}

/// IPv4 addresses of `host` from DoH; empty when nobody answered.
pub async fn lookup(host: &str) -> Vec<IpAddr> {
    if let Some((ips, at)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(host).cloned()) {
        if at.elapsed() < KEEP {
            return ips;
        }
    }
    for server in SERVERS {
        let url = format!("{server}?name={host}&type=A");
        let Ok(resp) = client().get(&url).header("accept", "application/dns-json").send().await else { continue };
        let Ok(body) = resp.bytes().await else { continue };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else { continue };
        let ips: Vec<IpAddr> = v["Answer"].as_array().into_iter().flatten().filter(|a| a["type"] == 1).filter_map(|a| a["data"].as_str()?.parse().ok()).collect();
        if !ips.is_empty() {
            CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(host.to_owned(), (ips.clone(), Instant::now()));
            return ips;
        }
    }
    Vec::new()
}

/// The HTTP client's resolver: SoundCloud over DoH while the bypass is on, the system otherwise (or if DoH fails).
pub struct Resolver;

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            if super::bypass::doh_wanted(&host) {
                let ips = lookup(&host).await;
                if !ips.is_empty() {
                    let addrs: reqwest::dns::Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect::<Vec<_>>().into_iter());
                    return Ok(addrs);
                }
            }
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let addrs: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}
