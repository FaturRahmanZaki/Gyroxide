use anyhow::Result;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{fs, sync::RwLock};
use tracing::{info, warn};
use url::Url;

use crate::config::Config;

#[derive(Debug, Clone, PartialEq)]
pub enum ProxyType {
    Http,
    Socks5,
}

#[derive(Debug, Clone)]
pub struct ProxyEntry {
    pub url: String,
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub is_alive: bool,
    pub fail_count: u32,
}

impl ProxyEntry {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            return None;
        }

        let url_str = if raw.contains("://") {
            raw.to_string()
        } else {
            let parts: Vec<&str> = raw.split(':').collect();
            if parts.len() == 4 {
                format!("http://{}:{}@{}:{}", parts[2], parts[3], parts[0], parts[1])
            } else {
                format!("http://{}", raw)
            }
        };

        let url = Url::parse(&url_str).ok()?;

        let proxy_type = match url.scheme() {
            "socks5" | "socks5h" => ProxyType::Socks5,
            _ => ProxyType::Http,
        };

        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(match proxy_type {
            ProxyType::Socks5 => 1080,
            ProxyType::Http => 3128,
        });

        let username = if url.username().is_empty() {
            None
        } else {
            Some(url.username().to_string())
        };
        let password = url.password().map(ToString::to_string);

        Some(Self {
            url: url_str,
            proxy_type,
            host,
            port,
            username,
            password,
            is_alive: true,
            fail_count: 0,
        })
    }
}

pub struct ProxyManager {
    proxies: Arc<RwLock<Vec<ProxyEntry>>>,
    index: Arc<AtomicUsize>,
    pub config: Arc<Config>,
}

impl ProxyManager {
    pub async fn new(proxy_file: &str, config: Arc<Config>) -> Result<Self> {
        let proxies = load_proxies(proxy_file).await?;
        info!("Loaded {} proxies from '{}'", proxies.len(), proxy_file);

        Ok(Self {
            proxies: Arc::new(RwLock::new(proxies)),
            index: Arc::new(AtomicUsize::new(0)),
            config,
        })
    }

    pub async fn next_proxy(&self) -> Option<ProxyEntry> {
        let proxies = self.proxies.read().await;
        if proxies.is_empty() {
            return None;
        }

        let total = proxies.len();
        let start = self.index.fetch_add(1, Ordering::Relaxed);

        for i in 0..total {
            let idx = (start + i) % total;
            if proxies[idx].is_alive {
                return Some(proxies[idx].clone());
            }
        }

        warn!("All proxies are dead! Falling back to first proxy");
        Some(proxies[0].clone())
    }

    pub async fn mark_dead(&self, proxy_url: &str) {
        let max_failures = self.config.max_failures;
        let mut proxies = self.proxies.write().await;
        if let Some(p) = proxies.iter_mut().find(|p| p.url == proxy_url) {
            p.fail_count += 1;
            if p.fail_count >= max_failures && p.is_alive {
                p.is_alive = false;
                warn!("Proxy DEAD [{}] after {} failures", proxy_url, p.fail_count);
            }
        }
    }

    pub async fn mark_alive(&self, proxy_url: &str) {
        let mut proxies = self.proxies.write().await;
        if let Some(p) = proxies.iter_mut().find(|p| p.url == proxy_url) {
            p.is_alive = true;
            p.fail_count = 0;
            info!("Proxy ALIVE [{}]", proxy_url);
        }
    }

    pub async fn total_count(&self) -> usize {
        self.proxies.read().await.len()
    }

    pub async fn get_raw_proxy_text(&self) -> String {
        let p = std::path::Path::new(&self.config.proxy_file);
        if p.is_dir() {
            let direct_file = p.join("proxies.txt");
            if direct_file.is_file() {
                return fs::read_to_string(direct_file).await.unwrap_or_default();
            }
            let mut out = String::new();
            if let Ok(mut entries) = fs::read_dir(p).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        if let Ok(content) = fs::read_to_string(&entry_path).await {
                            if !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str(&content);
                        }
                    }
                }
            }
            return out;
        }

        fs::read_to_string(&self.config.proxy_file)
            .await
            .unwrap_or_default()
    }

    pub async fn update_proxies_from_text(&self, text: &str) -> Result<usize> {
        let parsed: Vec<ProxyEntry> = text
            .lines()
            .filter_map(ProxyEntry::parse)
            .collect();

        let p = std::path::Path::new(&self.config.proxy_file);
        let target_file = if p.is_dir() {
            p.join("proxies.txt")
        } else {
            p.to_path_buf()
        };

        if let Err(e) = fs::write(&target_file, text).await {
            warn!("Failed to write proxies to '{}': {}", target_file.display(), e);
        }

        let count = parsed.len();
        {
            let mut proxies = self.proxies.write().await;
            *proxies = parsed;
        }

        info!("Updated proxy list: {} proxies loaded", count);
        Ok(count)
    }

    pub async fn get_stats(&self) -> (usize, usize) {
        let proxies = self.proxies.read().await;
        let alive = proxies.iter().filter(|p| p.is_alive).count();
        (alive, proxies.len())
    }

    pub async fn get_proxy_entries(&self) -> Vec<ProxyEntry> {
        self.proxies.read().await.clone()
    }

    pub async fn start_health_check(self: Arc<Self>) {
        let interval = Duration::from_secs(self.config.health_check_interval_secs);
        loop {
            tokio::time::sleep(interval).await;
            self.clone().run_health_check().await;
        }
    }

    async fn run_health_check(self: Arc<Self>) {
        let dead_proxies: Vec<ProxyEntry> = {
            let proxies = self.proxies.read().await;
            proxies.iter().filter(|p| !p.is_alive).cloned().collect()
        };

        if dead_proxies.is_empty() {
            return;
        }

        info!("Health check: testing {} dead proxy/proxies", dead_proxies.len());

        let timeout = Duration::from_secs(self.config.request_timeout_secs);

        for proxy in dead_proxies {
            let url = proxy.url.clone();
            let alive = test_proxy_tcp(&proxy, timeout).await;
            if alive {
                self.mark_alive(&url).await;
            }
        }

        let (alive, total) = self.get_stats().await;
        info!("Health check done: {}/{} alive", alive, total);
    }
}

async fn test_proxy_tcp(proxy: &ProxyEntry, timeout: Duration) -> bool {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    matches!(
        tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await,
        Ok(Ok(_))
    )
}

async fn load_proxies(path: &str) -> Result<Vec<ProxyEntry>> {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        let mut proxies = Vec::new();
        if let Ok(mut entries) = fs::read_dir(p).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    if let Ok(content) = fs::read_to_string(&entry_path).await {
                        for line in content.lines() {
                            if let Some(entry) = ProxyEntry::parse(line) {
                                proxies.push(entry);
                            }
                        }
                    }
                }
            }
        }
        if proxies.is_empty() {
            warn!("Proxy directory '{}' contains no proxies or is empty", path);
        }
        return Ok(proxies);
    }

    let content = match fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => {
            let example = concat!(
                "http://1.2.3.4:8080\n",
                "socks5://user:password@5.6.7.8:1080\n",
            );
            if let Err(e) = fs::write(path, example).await {
                warn!("Could not create example proxies file at '{}': {}", path, e);
            } else {
                info!("Created example proxies file at '{}'", path);
            }
            return Ok(vec![]);
        }
    };

    let proxies: Vec<ProxyEntry> = content
        .lines()
        .filter_map(ProxyEntry::parse)
        .collect();

    if proxies.is_empty() {
        warn!("No valid proxies found in '{}'", path);
    }

    Ok(proxies)
}
