use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use prometheus::{
    register_counter_vec, register_gauge, register_histogram, CounterVec, Encoder, Gauge, Histogram,
    TextEncoder,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::proxy_manager::ProxyManager;

pub struct Metrics {
    pub requests_total: CounterVec,
    pub request_duration_seconds: Histogram,
    pub proxies_total: Gauge,
    pub proxies_alive: Gauge,
    pub proxies_dead: Gauge,
    pub proxy_failures_total: CounterVec,
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

        Ok(Self {
            requests_total,
            request_duration_seconds,
            proxies_total,
            proxies_alive,
            proxies_dead,
            proxy_failures_total,
        })
    }

    pub fn update_proxy_stats(&self, alive: usize, total: usize) {
        self.proxies_total.set(total as f64);
        self.proxies_alive.set(alive as f64);
        self.proxies_dead.set((total.saturating_sub(alive)) as f64);
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
        let (alive, total) = pm.get_stats().await;
        m.update_proxy_stats(alive, total);

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
        let (alive, total) = pm.get_stats().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let html = render_dashboard_html(alive, total, &proxy_text, None);

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
        let new_content = if raw_str.starts_with("proxies=") {
            let encoded = &raw_str[8..];
            urlencoding_decode(encoded)
        } else {
            raw_str
        };

        let message = match pm.update_proxies_from_text(&new_content).await {
            Ok(count) => {
                let (alive, total) = pm.get_stats().await;
                m.update_proxy_stats(alive, total);
                format!("Success! {} valid proxies updated and reloaded.", count)
            }
            Err(e) => format!("Failed to update: {}", e),
        };

        let (alive, total) = pm.get_stats().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let html = render_dashboard_html(alive, total, &proxy_text, Some(&message));

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


fn render_dashboard_html(alive: usize, total: usize, proxies_txt: &str, alert: Option<&str>) -> String {
    let alert_box = match alert {
        Some(msg) => format!(
            r#"<div style="padding: 12px 16px; margin-bottom: 20px; border-radius: 8px; background: #064e3b; color: #6ee7b7; border: 1px solid #059669; font-weight: 500;">{}</div>"#,
            msg
        ),
        None => "".to_string(),
    };

    let escaped_proxies = html_escape(proxies_txt);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Gyroxide Proxy Manager</title>
  <style>
    :root {{
      --bg: #090d16;
      --card-bg: #111827;
      --border: #1f2937;
      --accent: #3b82f6;
      --accent-hover: #2563eb;
      --text: #f9fafb;
      --text-muted: #9ca3af;
      --green: #10b981;
      --red: #ef4444;
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: var(--bg);
      color: var(--text);
      padding: 30px 20px;
    }}
    .container {{
      max-width: 900px;
      margin: 0 auto;
    }}
    header {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 30px;
      padding-bottom: 20px;
      border-bottom: 1px solid var(--border);
    }}
    h1 {{
      font-size: 26px;
      font-weight: 700;
      letter-spacing: -0.5px;
      display: flex;
      align-items: center;
      gap: 10px;
    }}
    .badge {{
      font-size: 12px;
      font-weight: 600;
      padding: 4px 10px;
      border-radius: 20px;
      background: rgba(59, 130, 246, 0.2);
      color: var(--accent);
      border: 1px solid rgba(59, 130, 246, 0.3);
    }}
    .stats-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 16px;
      margin-bottom: 25px;
    }}
    .stat-card {{
      background: var(--card-bg);
      border: 1px solid var(--border);
      border-radius: 12px;
      padding: 20px;
    }}
    .stat-label {{
      font-size: 13px;
      color: var(--text-muted);
      text-transform: uppercase;
      font-weight: 600;
      letter-spacing: 0.5px;
      margin-bottom: 6px;
    }}
    .stat-value {{
      font-size: 28px;
      font-weight: 700;
    }}
    .val-alive {{ color: var(--green); }}
    .val-dead {{ color: var(--red); }}
    .card {{
      background: var(--card-bg);
      border: 1px solid var(--border);
      border-radius: 12px;
      padding: 24px;
    }}
    .card-title {{
      font-size: 18px;
      font-weight: 600;
      margin-bottom: 8px;
    }}
    .card-desc {{
      font-size: 14px;
      color: var(--text-muted);
      margin-bottom: 18px;
      line-height: 1.5;
    }}
    textarea {{
      width: 100%;
      height: 380px;
      background: #030712;
      color: #e5e7eb;
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 16px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      font-size: 13.5px;
      line-height: 1.6;
      resize: vertical;
      white-space: pre;
    }}
    textarea:focus {{
      outline: none;
      border-color: var(--accent);
      box-shadow: 0 0 0 2px rgba(59, 130, 246, 0.2);
    }}
    .actions {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-top: 18px;
    }}
    .btn {{
      background: var(--accent);
      color: white;
      border: none;
      padding: 12px 26px;
      font-size: 15px;
      font-weight: 600;
      border-radius: 8px;
      cursor: pointer;
      transition: background 0.15s ease;
    }}
    .btn:hover {{ background: var(--accent-hover); }}
    .metrics-link {{
      color: var(--text-muted);
      text-decoration: none;
      font-size: 14px;
      transition: color 0.15s;
    }}
    .metrics-link:hover {{ color: var(--accent); }}
  </style>
</head>
<body>
  <div class="container">
    <header>
      <h1>Gyroxide 🌀 <span class="badge">Auto Rotate Proxy</span></h1>
      <a href="/metrics" target="_blank" class="metrics-link">Prometheus /metrics ↗</a>
    </header>

    {alert_box}

    <div class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">Total Proxies</div>
        <div class="stat-value">{total}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Alive / Active</div>
        <div class="stat-value val-alive">{alive}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">Dead / Down</div>
        <div class="stat-value val-dead">{dead}</div>
      </div>
    </div>

    <div class="card">
      <div class="card-title">Manage Proxy List</div>
      <div class="card-desc">
        Paste bulk proxy kamu di bawah ini (1 baris per proxy). Mendukung format <code>ip:port:user:pass</code>, <code>http://...</code>, dan <code>socks5://...</code>. Klik tombol Simpan untuk auto reload tanpa restart!
      </div>
      <form method="POST" action="/admin">
        <textarea name="proxies" placeholder="Contoh:&#10;45.38.107.97:6014:piholwfu:9bjw4d9t5t6e&#10;socks5://user:pass@1.2.3.4:1080">{escaped_proxies}</textarea>
        <div class="actions">
          <span style="font-size: 13px; color: var(--text-muted);">Perubahan langsung aktif seketika.</span>
          <button type="submit" class="btn">Simpan & Reload Proxies</button>
        </div>
      </form>
    </div>
  </div>
</body>
</html>"#,
        alert_box = alert_box,
        total = total,
        alive = alive,
        dead = total.saturating_sub(alive),
        escaped_proxies = escaped_proxies
    )
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
