use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use prometheus::{
    register_counter_vec, register_gauge, register_gauge_vec, register_histogram, CounterVec, Encoder,
    Gauge, GaugeVec, Histogram, TextEncoder,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::dashboard::{self, AlertKind, Flash};
use crate::proxy_manager::{ProxyEntry, ProxyManager, ProxyType};

pub struct Metrics {
    pub requests_total: CounterVec,
    pub request_duration_seconds: Histogram,
    pub proxies_total: Gauge,
    pub proxies_alive: Gauge,
    pub proxies_dead: Gauge,
    pub proxy_failures_total: CounterVec,
    pub proxy_up: GaugeVec,
}

impl Metrics {
    pub fn new() -> Result<Self> {
        let requests_total = register_counter_vec!(
            "gyroxide_requests_total",
            "Total HTTP/CONNECT requests handled by Gyroxide",
            &["method", "status"]
        )?;

        let request_duration_seconds = register_histogram!(
            "gyroxide_request_duration_seconds",
            "Histogram of request latencies in seconds"
        )?;

        let proxies_total = register_gauge!(
            "gyroxide_proxies_total",
            "Total configured upstream proxies"
        )?;

        let proxies_alive = register_gauge!(
            "gyroxide_proxies_alive",
            "Number of currently alive upstream proxies"
        )?;

        let proxies_dead = register_gauge!(
            "gyroxide_proxies_dead",
            "Number of currently dead upstream proxies"
        )?;

        let proxy_failures_total = register_counter_vec!(
            "gyroxide_proxy_failures_total",
            "Total failures per upstream proxy",
            &["proxy"]
        )?;

        let proxy_up = register_gauge_vec!(
            "gyroxide_proxy_up",
            "Status of upstream proxy (1 = alive, 0 = dead)",
            &["address", "protocol"]
        )?;

        Ok(Self {
            requests_total,
            request_duration_seconds,
            proxies_total,
            proxies_alive,
            proxies_dead,
            proxy_failures_total,
            proxy_up,
        })
    }

    pub fn update_proxy_stats(&self, proxies: &[ProxyEntry]) {
        let total = proxies.len();
        let alive = proxies.iter().filter(|p| p.is_alive).count();
        self.proxies_total.set(total as f64);
        self.proxies_alive.set(alive as f64);
        self.proxies_dead.set((total.saturating_sub(alive)) as f64);

        self.proxy_up.reset();
        for p in proxies {
            let proto = match p.proxy_type {
                ProxyType::Http => "http",
                ProxyType::Socks5 => "socks5",
            };
            let addr = format!("{}:{}", p.host, p.port);
            let val = if p.is_alive { 1.0 } else { 0.0 };
            self.proxy_up.with_label_values(&[&addr, proto]).set(val);
        }
    }
}

pub async fn start_metrics_server(
    port: u16,
    proxy_manager: Arc<ProxyManager>,
    metrics: Arc<Metrics>,
) -> Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!("Gyroxide Web Dashboard & Metrics running on http://{}/admin", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let pm = proxy_manager.clone();
        let m = metrics.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let pm = pm.clone();
                let m = m.clone();
                async move {
                    handle_web_request(req, pm, m).await
                }
            });

            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                tracing::debug!("Dashboard connection error: {}", err);
            }
        });
    }
}

async fn handle_web_request(
    req: Request<Incoming>,
    pm: Arc<ProxyManager>,
    m: Arc<Metrics>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path();
    let method = req.method().clone();

    if method == Method::GET && path == "/metrics" {
        let entries = pm.get_proxy_entries().await;
        m.update_proxy_stats(&entries);

        let metric_families = prometheus::gather();
        let encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
            error!("Prometheus encode error: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from("Encode error")))
                .unwrap());
        }

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", encoder.format_type())
            .body(Full::new(Bytes::from(buffer)))
            .unwrap());
    }

    let config = &pm.config;
    if let (Some(u), Some(p)) = (&config.auth_username, &config.auth_password) {
        if !check_web_auth(req.headers(), u, p) {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("WWW-Authenticate", "Basic realm=\"Gyroxide Dashboard\"")
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(Full::new(Bytes::from("401 Unauthorized: Credentials required\n")))
                .unwrap());
        }
    }

    if (method == Method::GET || method == Method::HEAD) && (path == "/" || path == "/admin") {
        let entries = pm.get_proxy_entries().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let html = dashboard::render(&entries, &proxy_text, None);

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(html)))
            .unwrap());
    }

    if method == Method::POST && (path == "/admin" || path == "/api/proxies") {
        let body = match req.into_body().collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                return Ok(Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from(format!("Body error: {}", e))))
                    .unwrap());
            }
        };

        let raw_str = String::from_utf8_lossy(&body).to_string();
        let (proxies_input, mode_input) = parse_form_params(&raw_str);
        let mode = mode_input.unwrap_or_else(|| "append".to_string());
        let new_content = proxies_input.unwrap_or(raw_str);

        // `message` is the Indonesian fallback; `key` + `args` let the client
        // re-render the same banner in whichever language the user picked.
        let (message, kind, key, args): (String, AlertKind, &str, Vec<String>) =
            if mode == "replace" {
                match pm.update_proxies_from_text(&new_content).await {
                    Ok(count) => {
                        let entries = pm.get_proxy_entries().await;
                        m.update_proxy_stats(&entries);
                        (
                            format!("Berhasil! {} proxy disimpan dan di-reload.", count),
                            AlertKind::Success,
                            "alert.replaceOk",
                            vec![count.to_string()],
                        )
                    }
                    Err(e) => (
                        format!("Gagal memperbarui: {}", e),
                        AlertKind::Error,
                        "alert.replaceErr",
                        vec![e.to_string()],
                    ),
                }
            } else {
                match pm.append_proxies_from_text(&new_content).await {
                    Ok((added, total)) => {
                        let entries = pm.get_proxy_entries().await;
                        m.update_proxy_stats(&entries);
                        if added > 0 {
                            (
                                format!(
                                    "Berhasil menambahkan {} proxy baru! (Total sekarang: {} proxy)",
                                    added, total
                                ),
                                AlertKind::Success,
                                "alert.appendOk",
                                vec![added.to_string(), total.to_string()],
                            )
                        } else {
                            (
                                format!("Tidak ada proxy baru yang ditambahkan (semua proxy sudah ada dalam daftar atau format tidak valid). Total: {} proxy", total),
                                AlertKind::Info,
                                "alert.appendNone",
                                vec![total.to_string()],
                            )
                        }
                    }
                    Err(e) => (
                        format!("Gagal menambahkan: {}", e),
                        AlertKind::Error,
                        "alert.appendErr",
                        vec![e.to_string()],
                    ),
                }
            };

        let entries = pm.get_proxy_entries().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let flash = Flash {
            kind,
            key,
            args: &arg_refs,
            message: &message,
        };
        let html = dashboard::render(&entries, &proxy_text, Some(flash));

        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(html)))
            .unwrap());
    }

    Ok(Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .body(Full::new(Bytes::from("404 Not Found")))
        .unwrap())
}

fn parse_form_params(input: &str) -> (Option<String>, Option<String>) {
    let mut proxies = None;
    let mut mode = None;

    if !input.contains('=') {
        return (None, None);
    }

    for part in input.split('&') {
        let mut key_val = part.splitn(2, '=');
        let key = key_val.next().unwrap_or("");
        let val = key_val.next().unwrap_or("");
        if key == "proxies" {
            proxies = Some(urlencoding_decode(val));
        } else if key == "mode" {
            mode = Some(urlencoding_decode(val));
        }
    }

    (proxies, mode)
}

fn urlencoding_decode(input: &str) -> String {
    let replaced = input.replace('+', " ");
    let mut result = Vec::new();
    let bytes = replaced.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                result.push(val);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

fn check_web_auth(headers: &hyper::HeaderMap, expected_user: &str, expected_pass: &str) -> bool {
    let header_val = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        },
        None => return false,
    };

    if !header_val.starts_with("Basic ") {
        return false;
    }

    let b64 = &header_val[6..].trim();
    use base64::Engine;
    let decoded = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(d) => d,
        Err(_) => return false,
    };

    let creds = match String::from_utf8(decoded) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let mut parts = creds.splitn(2, ':');
    let u = parts.next().unwrap_or("");
    let p = parts.next().unwrap_or("");

    u == expected_user && p == expected_pass
}


