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
        let html = render_dashboard_html(&entries, &proxy_text, None);

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
                let entries = pm.get_proxy_entries().await;
                m.update_proxy_stats(&entries);
                format!("Success! {} valid proxies updated and reloaded.", count)
            }
            Err(e) => format!("Failed to update: {}", e),
        };

        let entries = pm.get_proxy_entries().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let html = render_dashboard_html(&entries, &proxy_text, Some(&message));

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


fn render_dashboard_html(entries: &[ProxyEntry], proxies_txt: &str, alert: Option<&str>) -> String {
    let alert_box = match alert {
        Some(msg) => format!(
            r#"<div style="padding: 12px 16px; margin-bottom: 20px; border-radius: 8px; background: #064e3b; color: #6ee7b7; border: 1px solid #059669; font-weight: 500;">{}</div>"#,
            msg
        ),
        None => "".to_string(),
    };

    let total = entries.len();
    let alive = entries.iter().filter(|p| p.is_alive).count();
    let dead = total.saturating_sub(alive);

    let mut table_rows = String::new();
    for (i, p) in entries.iter().enumerate() {
        let proto = match p.proxy_type {
            ProxyType::Http => "<span class=\"badge-pill badge-http\">HTTP</span>",
            ProxyType::Socks5 => "<span class=\"badge-pill badge-socks5\">SOCKS5</span>",
        };
        let auth = if let Some(ref u) = p.username {
            format!("<span class=\"badge-pill badge-auth\" title=\"User: {}\">Auth</span>", html_escape(u))
        } else {
            "<span class=\"badge-pill badge-noauth\">No Auth</span>".to_string()
        };
        let status = if p.is_alive {
            "<span class=\"status-pill status-alive\">● ALIVE</span>"
        } else {
            "<span class=\"status-pill status-dead\">● DEAD</span>"
        };
        let host_port = format!("{}:{}", p.host, p.port);
        let escaped_addr = html_escape(&host_port);

        table_rows.push_str(&format!(
            r#"<tr>
  <td class="col-num">{}</td>
  <td class="col-addr"><code>{}</code></td>
  <td>{}</td>
  <td>{}</td>
  <td>{}</td>
  <td class="col-fail">{}</td>
  <td class="col-act"><button type="button" class="btn-del" onclick="deleteProxyByAddress('{}')">🗑️ Hapus</button></td>
</tr>"#,
            i + 1,
            escaped_addr,
            proto,
            auth,
            status,
            p.fail_count,
            escaped_addr
        ));
    }

    if table_rows.is_empty() {
        table_rows = r#"<tr><td colspan="7" class="empty-state">Belum ada proxy di proxies.txt. Tambahkan di form atas atau buka Bulk Editor.</td></tr>"#.to_string();
    }

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
      --bg: #0b0f19;
      --card-bg: #111827;
      --card-border: #1f2937;
      --accent: #3b82f6;
      --accent-hover: #2563eb;
      --text: #f9fafb;
      --text-muted: #9ca3af;
      --green: #10b981;
      --green-bg: rgba(16, 185, 129, 0.15);
      --red: #ef4444;
      --red-bg: rgba(239, 68, 68, 0.15);
      --purple: #8b5cf6;
      --purple-bg: rgba(139, 92, 246, 0.15);
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: var(--bg);
      color: var(--text);
      padding: 30px 20px;
    }}
    .container {{
      max-width: 1050px;
      margin: 0 auto;
    }}
    header {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 24px;
      padding-bottom: 18px;
      border-bottom: 1px solid var(--card-border);
    }}
    h1 {{
      font-size: 24px;
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
    .metrics-link {{
      color: var(--text-muted);
      text-decoration: none;
      font-size: 14px;
      transition: color 0.15s;
    }}
    .metrics-link:hover {{ color: var(--accent); }}
    .stats-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 16px;
      margin-bottom: 24px;
    }}
    .stat-card {{
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 12px;
      padding: 18px 20px;
    }}
    .stat-label {{
      font-size: 12px;
      color: var(--text-muted);
      text-transform: uppercase;
      font-weight: 600;
      letter-spacing: 0.5px;
      margin-bottom: 6px;
    }}
    .stat-value {{
      font-size: 26px;
      font-weight: 700;
    }}
    .val-alive {{ color: var(--green); }}
    .val-dead {{ color: var(--red); }}
    .panel {{
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 12px;
      padding: 20px 24px;
      margin-bottom: 24px;
    }}
    .toolbar {{
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 18px;
    }}
    .search-input {{
      flex: 1;
      min-width: 240px;
      background: #030712;
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 10px 14px;
      color: var(--text);
      font-size: 14px;
    }}
    .search-input:focus {{
      outline: none;
      border-color: var(--accent);
    }}
    .quick-add-wrap {{
      display: flex;
      gap: 8px;
      flex: 1.5;
      min-width: 320px;
    }}
    .quick-input {{
      flex: 1;
      background: #030712;
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 10px 14px;
      color: var(--text);
      font-size: 13.5px;
      font-family: monospace;
    }}
    .quick-input:focus {{
      outline: none;
      border-color: var(--accent);
    }}
    .btn {{
      background: var(--accent);
      color: white;
      border: none;
      padding: 10px 18px;
      font-size: 14px;
      font-weight: 600;
      border-radius: 8px;
      cursor: pointer;
      transition: background 0.15s ease;
      white-space: nowrap;
    }}
    .btn:hover {{ background: var(--accent-hover); }}
    .btn-secondary {{
      background: #1f2937;
      color: #e5e7eb;
      border: 1px solid #374151;
    }}
    .btn-secondary:hover {{ background: #374151; }}
    .btn-del {{
      background: rgba(239, 68, 68, 0.15);
      color: #f87171;
      border: 1px solid rgba(239, 68, 68, 0.3);
      padding: 6px 12px;
      font-size: 12.5px;
      font-weight: 600;
      border-radius: 6px;
      cursor: pointer;
      transition: all 0.15s;
    }}
    .btn-del:hover {{
      background: #ef4444;
      color: white;
    }}
    table {{
      width: 100%;
      border-collapse: collapse;
      font-size: 14px;
    }}
    th {{
      text-align: left;
      padding: 12px 14px;
      font-size: 12px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-muted);
      border-bottom: 1px solid var(--card-border);
    }}
    td {{
      padding: 13px 14px;
      border-bottom: 1px solid rgba(31, 41, 55, 0.6);
      vertical-align: middle;
    }}
    tr:hover td {{
      background: rgba(255, 255, 255, 0.02);
    }}
    .col-num {{ color: var(--text-muted); font-size: 12px; width: 40px; }}
    .col-addr code {{
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      font-size: 13.5px;
      color: #93c5fd;
    }}
    .col-fail {{ color: var(--text-muted); }}
    .col-act {{ text-align: right; width: 90px; }}
    .badge-pill {{
      display: inline-block;
      font-size: 11.5px;
      font-weight: 600;
      padding: 3px 8px;
      border-radius: 6px;
    }}
    .badge-http {{ background: rgba(59, 130, 246, 0.2); color: #60a5fa; }}
    .badge-socks5 {{ background: var(--purple-bg); color: #c084fc; }}
    .badge-auth {{ background: rgba(245, 158, 11, 0.15); color: #fbbf24; }}
    .badge-noauth {{ background: rgba(107, 114, 128, 0.2); color: #9ca3af; }}
    .status-pill {{
      display: inline-block;
      font-size: 11.5px;
      font-weight: 700;
      padding: 3px 10px;
      border-radius: 12px;
      letter-spacing: 0.5px;
    }}
    .status-alive {{ background: var(--green-bg); color: var(--green); }}
    .status-dead {{ background: var(--red-bg); color: var(--red); }}
    .empty-state {{
      text-align: center;
      padding: 36px !important;
      color: var(--text-muted);
    }}
    .bulk-section {{
      display: none;
      margin-top: 20px;
      border-top: 1px solid var(--card-border);
      padding-top: 20px;
    }}
    textarea {{
      width: 100%;
      height: 300px;
      background: #030712;
      color: #e5e7eb;
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 14px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      font-size: 13px;
      line-height: 1.6;
      resize: vertical;
      white-space: pre;
      margin-bottom: 12px;
    }}
    textarea:focus {{
      outline: none;
      border-color: var(--accent);
    }}
    .bulk-actions {{
      display: flex;
      justify-content: space-between;
      align-items: center;
    }}
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

    <div class="panel">
      <div class="toolbar">
        <input type="text" id="searchInput" class="search-input" placeholder="🔍 Cari IP, port, atau protocol..." oninput="filterProxies()">
        <div class="quick-add-wrap">
          <input type="text" id="quickInput" class="quick-input" placeholder="Tambah: ip:port:user:pass" onkeydown="if(event.key==='Enter')quickAddProxy()">
          <button type="button" class="btn" onclick="quickAddProxy()">+ Tambah</button>
        </div>
        <button type="button" id="toggleBulkBtn" class="btn btn-secondary" onclick="toggleBulkEditor()">📝 Bulk Editor</button>
      </div>

      <div style="overflow-x: auto;">
        <table id="proxyTable">
          <thead>
            <tr>
              <th>#</th>
              <th>Proxy Address</th>
              <th>Protocol</th>
              <th>Auth</th>
              <th>Status</th>
              <th>Failures</th>
              <th style="text-align: right;">Aksi</th>
            </tr>
          </thead>
          <tbody>
            {table_rows}
          </tbody>
        </table>
      </div>

      <div id="bulkSection" class="bulk-section">
        <h3 style="font-size: 16px; margin-bottom: 8px;">Raw Bulk Editor</h3>
        <p style="font-size: 13px; color: var(--text-muted); margin-bottom: 12px;">Edit isi proxies.txt secara langsung (1 baris per proxy). Mendukung format <code>ip:port:user:pass</code> atau <code>socks5://...</code></p>
        <form id="proxyForm" method="POST" action="/admin">
          <textarea id="rawProxies" name="proxies">{escaped_proxies}</textarea>
          <div class="bulk-actions">
            <span style="font-size: 13px; color: var(--text-muted);">Perubahan langsung disimpan ke proxies.txt & di-reload.</span>
            <button type="submit" class="btn">💾 Simpan & Reload Semua</button>
          </div>
        </form>
      </div>
    </div>
  </div>

  <script>
    function deleteProxyByAddress(addr) {{
      if (!confirm('Hapus proxy ' + addr + ' dari daftar?')) return;
      const textarea = document.getElementById('rawProxies');
      const lines = textarea.value.split('\n');
      let removed = false;
      const filtered = [];
      for (const line of lines) {{
        const trimmed = line.trim();
        if (!removed && trimmed.length > 0 && trimmed.includes(addr)) {{
          removed = true;
          continue;
        }}
        filtered.push(line);
      }}
      textarea.value = filtered.join('\n');
      document.getElementById('proxyForm').submit();
    }}

    function quickAddProxy() {{
      const input = document.getElementById('quickInput');
      const val = input.value.trim();
      if (!val) {{
        alert('Silakan masukkan format proxy, contoh: ip:port:user:pass');
        return;
      }}
      const textarea = document.getElementById('rawProxies');
      textarea.value = (textarea.value.trim() ? textarea.value.trim() + '\n' : '') + val;
      document.getElementById('proxyForm').submit();
    }}

    function filterProxies() {{
      const q = document.getElementById('searchInput').value.toLowerCase();
      const rows = document.querySelectorAll('#proxyTable tbody tr');
      rows.forEach(r => {{
        const text = r.innerText.toLowerCase();
        r.style.display = text.includes(q) ? '' : 'none';
      }});
    }}

    function toggleBulkEditor() {{
      const section = document.getElementById('bulkSection');
      const btn = document.getElementById('toggleBulkBtn');
      if (section.style.display === 'none' || section.style.display === '') {{
        section.style.display = 'block';
        btn.innerText = '✕ Tutup Bulk Editor';
      }} else {{
        section.style.display = 'none';
        btn.innerText = '📝 Bulk Editor';
      }}
    }}
  </script>
</body>
</html>"#,
        alert_box = alert_box,
        total = total,
        alive = alive,
        dead = dead,
        table_rows = table_rows,
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
