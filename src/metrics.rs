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
            r#"<div style="padding: 12px 16px; margin-bottom: 20px; border-radius: 8px; background: rgba(16, 185, 129, 0.15); color: #6ee7b7; border: 1px solid rgba(16, 185, 129, 0.3); font-weight: 500;">{}</div>"#,
            msg
        ),
        None => "".to_string(),
    };

    let total = entries.len();
    let alive = entries.iter().filter(|p| p.is_alive).count();
    let dead = total.saturating_sub(alive);

    let mut table_rows = String::new();
    let lines: Vec<&str> = proxies_txt
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .collect();

    let mut used_lines = std::collections::HashSet::new();

    for (i, p) in entries.iter().enumerate() {
        let proto_badge = match p.proxy_type {
            ProxyType::Http => "<span class=\"badge-pill badge-http\">HTTP</span>",
            ProxyType::Socks5 => "<span class=\"badge-pill badge-socks5\">SOCKS5</span>",
        };
        let proto_str = match p.proxy_type {
            ProxyType::Http => "http",
            ProxyType::Socks5 => "socks5",
        };
        let auth = if let Some(ref u) = p.username {
            format!("<span class=\"badge-pill badge-auth\" title=\"User: {}\">Auth</span>", html_escape(u))
        } else {
            "<span class=\"badge-pill badge-noauth\">No Auth</span>".to_string()
        };
        let status_badge = if p.is_alive {
            "<span class=\"status-pill status-alive\"><span class=\"status-dot\"></span>ALIVE</span>"
        } else {
            "<span class=\"status-pill status-dead\"><span class=\"status-dot\"></span>DEAD</span>"
        };
        let status_str = if p.is_alive { "alive" } else { "dead" };
        let host_port = format!("{}:{}", p.host, p.port);
        let escaped_addr = html_escape(&host_port);

        let mut raw_line = host_port.clone();
        for (idx, line) in lines.iter().enumerate() {
            if !used_lines.contains(&idx) && line.contains(&p.host) && line.contains(&p.port.to_string()) {
                used_lines.insert(idx);
                raw_line = line.to_string();
                break;
            }
        }
        let escaped_raw = html_escape(&raw_line);

        table_rows.push_str(&format!(
            r#"<tr class="proxy-row" data-index="{idx}" data-addr="{addr}" data-proto="{proto}" data-status="{status}" data-raw="{raw}">
  <td class="col-check"><input type="checkbox" class="row-checkbox" value="{addr}" onchange="handleRowCheckboxChange(this, event)"></td>
  <td class="col-num">{num}</td>
  <td class="col-addr"><code>{addr}</code></td>
  <td>{proto_badge}</td>
  <td>{auth}</td>
  <td>{status_badge}</td>
  <td class="col-fail">{fail_count}</td>
  <td class="col-act">
    <button type="button" class="btn-icon" title="Salin Proxy" onclick="copyRowProxy(this)">📋</button>
    <button type="button" class="btn-del" title="Hapus Proxy" onclick="deleteRowProxy(this)">🗑️</button>
  </td>
</tr>"#,
            idx = i,
            num = i + 1,
            addr = escaped_addr,
            proto = proto_str,
            status = status_str,
            raw = escaped_raw,
            proto_badge = proto_badge,
            auth = auth,
            status_badge = status_badge,
            fail_count = p.fail_count
        ));
    }

    if table_rows.is_empty() {
        table_rows = r#"<tr id="emptyRow"><td colspan="8" class="empty-state">Belum ada proxy di proxies.txt. Tambahkan di form atas atau buka Bulk Editor.</td></tr>"#.to_string();
    } else {
        table_rows.push_str(r#"<tr id="emptyRow" style="display: none;"><td colspan="8" class="empty-state">Tidak ada proxy yang cocok dengan filter atau pencarian.</td></tr>"#);
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
      --bg: #090d16;
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
      --amber: #f59e0b;
      --amber-bg: rgba(245, 158, 11, 0.15);
    }}
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: var(--bg);
      color: var(--text);
      padding: 30px 20px;
    }}
    .container {{
      max-width: 1100px;
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
      flex-direction: column;
      gap: 12px;
      margin-bottom: 18px;
    }}
    .toolbar-filters {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      align-items: center;
    }}
    .toolbar-actions {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      align-items: center;
      justify-content: space-between;
    }}
    .search-input {{
      flex: 1;
      min-width: 220px;
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
    .select-control {{
      background: #030712;
      border: 1px solid var(--card-border);
      border-radius: 8px;
      padding: 10px 12px;
      color: var(--text);
      font-size: 13.5px;
      cursor: pointer;
    }}
    .select-control:focus {{
      outline: none;
      border-color: var(--accent);
    }}
    .quick-add-wrap {{
      display: flex;
      gap: 8px;
      flex: 1;
      min-width: 280px;
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
    .selection-bar {{
      display: none;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
      padding: 12px 18px;
      background: rgba(30, 41, 59, 0.95);
      border: 1px solid rgba(59, 130, 246, 0.4);
      border-radius: 10px;
      margin-bottom: 16px;
      box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    }}
    .selection-bar.active {{
      display: flex;
    }}
    .selection-bar-left, .selection-bar-right {{
      display: flex;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
    }}
    .selected-badge {{
      display: inline-flex;
      align-items: center;
      background: rgba(59, 130, 246, 0.2);
      color: #93c5fd;
      border: 1px solid rgba(59, 130, 246, 0.4);
      padding: 5px 12px;
      border-radius: 20px;
      font-size: 13px;
      font-weight: 600;
    }}
    .btn-batch-del {{
      background: rgba(239, 68, 68, 0.2);
      color: #fca5a5;
      border: 1px solid rgba(239, 68, 68, 0.4);
      padding: 7px 14px;
      font-size: 13px;
      font-weight: 600;
      border-radius: 6px;
      cursor: pointer;
      transition: all 0.15s;
    }}
    .btn-batch-del:hover {{
      background: #ef4444;
      color: white;
    }}
    .btn-batch-action {{
      background: #1e293b;
      color: #e2e8f0;
      border: 1px solid #334155;
      padding: 7px 12px;
      font-size: 13px;
      font-weight: 500;
      border-radius: 6px;
      cursor: pointer;
      transition: all 0.15s;
    }}
    .btn-batch-action:hover {{
      background: #334155;
      color: white;
    }}
    input[type="checkbox"] {{
      width: 17px;
      height: 17px;
      cursor: pointer;
      accent-color: var(--accent);
      border-radius: 4px;
    }}
    .col-check {{
      width: 44px;
      text-align: center;
    }}
    .col-num {{ color: var(--text-muted); font-size: 12px; width: 40px; }}
    .col-addr code {{
      font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      font-size: 13.5px;
      color: #93c5fd;
    }}
    .col-fail {{ color: var(--text-muted); }}
    .col-act {{ text-align: right; width: 100px; white-space: nowrap; }}
    .btn-icon {{
      background: transparent;
      border: 1px solid rgba(255, 255, 255, 0.12);
      color: var(--text-muted);
      padding: 5px 8px;
      font-size: 13px;
      border-radius: 6px;
      cursor: pointer;
      transition: all 0.15s;
      margin-right: 4px;
    }}
    .btn-icon:hover {{
      background: rgba(255, 255, 255, 0.1);
      color: var(--text);
    }}
    .btn-del {{
      background: rgba(239, 68, 68, 0.15);
      color: #f87171;
      border: 1px solid rgba(239, 68, 68, 0.3);
      padding: 5px 9px;
      font-size: 13px;
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
      padding: 12px 14px;
      border-bottom: 1px solid rgba(31, 41, 55, 0.6);
      vertical-align: middle;
    }}
    tr.proxy-row:hover td {{
      background: rgba(255, 255, 255, 0.02);
    }}
    tr.selected-row {{
      background: rgba(59, 130, 246, 0.08) !important;
      border-left: 3px solid var(--accent);
    }}
    .badge-pill {{
      display: inline-block;
      font-size: 11.5px;
      font-weight: 600;
      padding: 3px 8px;
      border-radius: 6px;
    }}
    .badge-http {{ background: rgba(59, 130, 246, 0.2); color: #60a5fa; }}
    .badge-socks5 {{ background: var(--purple-bg); color: #c084fc; }}
    .badge-auth {{ background: var(--amber-bg); color: #fbbf24; }}
    .badge-noauth {{ background: rgba(107, 114, 128, 0.2); color: #9ca3af; }}
    .status-pill {{
      display: inline-flex;
      align-items: center;
      font-size: 11.5px;
      font-weight: 700;
      padding: 3px 10px;
      border-radius: 12px;
      letter-spacing: 0.5px;
    }}
    .status-dot {{
      display: inline-block;
      width: 6px;
      height: 6px;
      border-radius: 50%;
      margin-right: 6px;
    }}
    .status-alive {{ background: var(--green-bg); color: var(--green); }}
    .status-alive .status-dot {{
      background-color: var(--green);
      box-shadow: 0 0 6px var(--green);
    }}
    .status-dead {{ background: var(--red-bg); color: var(--red); }}
    .status-dead .status-dot {{
      background-color: var(--red);
      box-shadow: 0 0 6px var(--red);
    }}
    .empty-state {{
      text-align: center;
      padding: 36px !important;
      color: var(--text-muted);
    }}
    .pagination-container {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      flex-wrap: wrap;
      gap: 12px;
      padding-top: 16px;
      margin-top: 8px;
      border-top: 1px solid var(--card-border);
    }}
    .pagination-info {{
      font-size: 13px;
      color: var(--text-muted);
    }}
    .pagination-nav {{
      display: flex;
      align-items: center;
      gap: 5px;
    }}
    .page-btn {{
      min-width: 32px;
      height: 32px;
      padding: 0 8px;
      background: #111827;
      color: #d1d5db;
      border: 1px solid var(--card-border);
      border-radius: 6px;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      transition: all 0.15s;
    }}
    .page-btn:hover:not(:disabled) {{
      background: #1f2937;
      color: #f3f4f6;
      border-color: #374151;
    }}
    .page-btn.active {{
      background: var(--accent);
      color: white;
      border-color: var(--accent);
      font-weight: 700;
    }}
    .page-btn:disabled {{
      opacity: 0.35;
      cursor: not-allowed;
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
    .toast-msg {{
      position: fixed;
      bottom: 24px;
      right: 24px;
      background: #1e293b;
      color: #f8fafc;
      border: 1px solid #3b82f6;
      padding: 12px 20px;
      border-radius: 8px;
      font-size: 13.5px;
      box-shadow: 0 10px 25px rgba(0, 0, 0, 0.5);
      transform: translateY(100px);
      opacity: 0;
      transition: all 0.25s ease;
      z-index: 9999;
      pointer-events: none;
    }}
    .toast-msg.show {{
      transform: translateY(0);
      opacity: 1;
    }}
  </style>
</head>
<body>
  <div class="container">
    <header>
      <h1>Gyroxide 🌀 <span class="badge">Auto Rotate Proxy</span></h1>
      <a href="http://43.173.19.116:3000/d/adzs2kz/gyroxide-proxy-monitor?from=now-15m&to=now&timezone=browser&refresh=10s" target="_blank" class="metrics-link">Go to Monitoring ↗</a>
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
        <div class="toolbar-filters">
          <input type="text" id="searchInput" class="search-input" placeholder="🔍 Cari IP, port, atau kata kunci..." oninput="applyFiltersAndPagination(1)">
          <select id="protoFilter" class="select-control" onchange="applyFiltersAndPagination(1)">
            <option value="all">Semua Protokol</option>
            <option value="http">HTTP</option>
            <option value="socks5">SOCKS5</option>
          </select>
          <select id="statusFilter" class="select-control" onchange="applyFiltersAndPagination(1)">
            <option value="all">Semua Status</option>
            <option value="alive">Alive</option>
            <option value="dead">Dead</option>
          </select>
          <select id="pageSizeSelect" class="select-control" onchange="handlePageSizeChange(this.value)">
            <option value="10">10 / hal</option>
            <option value="25" selected>25 / hal</option>
            <option value="50">50 / hal</option>
            <option value="100">100 / hal</option>
          </select>
        </div>
        <div class="toolbar-actions">
          <div class="quick-add-wrap">
            <input type="text" id="quickInput" class="quick-input" placeholder="Tambah: ip:port:user:pass" onkeydown="if(event.key==='Enter')quickAddProxy()">
            <button type="button" class="btn" onclick="quickAddProxy()">+ Tambah</button>
          </div>
          <button type="button" id="toggleBulkBtn" class="btn btn-secondary" onclick="toggleBulkEditor()">📝 Bulk Editor</button>
        </div>
      </div>

      <div id="selectionBar" class="selection-bar">
        <div class="selection-bar-left">
          <span id="selectedCountText" class="selected-badge">0 dipilih</span>
          <button type="button" class="btn-batch-action" onclick="selectAllFiltered()">Pilih Semua Hasil Filter</button>
          <button type="button" class="btn-batch-action" onclick="copySelectedProxies()">📋 Salin Terpilih</button>
          <button type="button" class="btn-batch-action" onclick="exportSelectedProxies()">📥 Export .txt</button>
        </div>
        <div class="selection-bar-right">
          <button type="button" class="btn-batch-del" onclick="deleteSelectedProxies()">🗑️ Hapus Terpilih</button>
          <button type="button" class="btn-batch-action" onclick="clearAllSelection()">✕ Batal</button>
        </div>
      </div>

      <div style="overflow-x: auto;">
        <table id="proxyTable">
          <thead>
            <tr>
              <th class="col-check">
                <input type="checkbox" id="selectAllCheckbox" title="Pilih Semua di Halaman Ini" onchange="toggleSelectAll(this)">
              </th>
              <th class="col-num">#</th>
              <th>Proxy Address</th>
              <th>Protocol</th>
              <th>Auth</th>
              <th>Status</th>
              <th>Failures</th>
              <th style="text-align: right; width: 110px;">Aksi</th>
            </tr>
          </thead>
          <tbody>
            {table_rows}
          </tbody>
        </table>
      </div>

      <div class="pagination-container">
        <div id="paginationInfo" class="pagination-info">Menampilkan 0 dari 0 proxy</div>
        <div class="pagination-nav">
          <button type="button" id="firstPageBtn" class="page-btn" onclick="applyFiltersAndPagination(1)" title="Halaman Pertama">«</button>
          <button type="button" id="prevPageBtn" class="page-btn" onclick="applyFiltersAndPagination(currentPage - 1)" title="Halaman Sebelumnya">‹</button>
          <div id="paginationPages" style="display: inline-flex; gap: 4px;"></div>
          <button type="button" id="nextPageBtn" class="page-btn" onclick="applyFiltersAndPagination(currentPage + 1)" title="Halaman Berikutnya">›</button>
          <button type="button" id="lastPageBtn" class="page-btn" onclick="applyFiltersAndPagination(Math.ceil(getFilteredRows().length / pageSize))" title="Halaman Terakhir">»</button>
        </div>
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

  <div id="toastNotification" class="toast-msg"></div>

  <script>
    let currentPage = 1;
    let pageSize = 25;
    const selectedAddrs = new Set();
    const selectedRaws = new Map();
    let toastTimer = null;

    function showToast(msg) {{
      const toast = document.getElementById('toastNotification');
      if (!toast) return;
      toast.textContent = msg;
      toast.classList.add('show');
      if (toastTimer) clearTimeout(toastTimer);
      toastTimer = setTimeout(() => {{
        toast.classList.remove('show');
      }}, 2500);
    }}

    function getRows() {{
      return Array.from(document.querySelectorAll('.proxy-row'));
    }}

    function getFilteredRows() {{
      const q = document.getElementById('searchInput').value.trim().toLowerCase();
      const proto = document.getElementById('protoFilter').value;
      const status = document.getElementById('statusFilter').value;
      const allRows = getRows();

      return allRows.filter(r => {{
        const rProto = r.getAttribute('data-proto');
        const rStatus = r.getAttribute('data-status');
        const rAddr = r.getAttribute('data-addr').toLowerCase();
        const rRaw = r.getAttribute('data-raw').toLowerCase();

        if (proto !== 'all' && rProto !== proto) return false;
        if (status !== 'all' && rStatus !== status) return false;
        if (q) {{
          const text = r.innerText.toLowerCase();
          if (!text.includes(q) && !rAddr.includes(q) && !rRaw.includes(q)) {{
            return false;
          }}
        }}
        return true;
      }});
    }}

    function applyFiltersAndPagination(page) {{
      if (page) currentPage = page;
      const filtered = getFilteredRows();
      const total = filtered.length;
      const totalPages = Math.max(1, Math.ceil(total / pageSize));
      if (currentPage > totalPages) currentPage = totalPages;
      if (currentPage < 1) currentPage = 1;

      const startIdx = (currentPage - 1) * pageSize;
      const endIdx = Math.min(startIdx + pageSize, total);

      const allRows = getRows();
      allRows.forEach(r => r.style.display = 'none');

      for (let i = startIdx; i < endIdx; i++) {{
        filtered[i].style.display = '';
      }}

      const emptyRow = document.getElementById('emptyRow');
      if (emptyRow) {{
        emptyRow.style.display = total === 0 ? '' : 'none';
      }}

      updatePaginationControls(total, totalPages, startIdx, endIdx);
      updateSelectAllCheckboxState(filtered.slice(startIdx, endIdx));
      updateSelectionUI();
    }}

    function handlePageSizeChange(val) {{
      pageSize = parseInt(val, 10) || 25;
      applyFiltersAndPagination(1);
    }}

    function updatePaginationControls(total, totalPages, startIdx, endIdx) {{
      const infoEl = document.getElementById('paginationInfo');
      const allCount = getRows().length;
      if (total === 0) {{
        infoEl.textContent = 'Menampilkan 0 dari 0 proxy';
      }} else {{
        let text = 'Menampilkan ' + (startIdx + 1) + '-' + endIdx + ' dari ' + total + ' proxy';
        if (total !== allCount) {{
          text += ' (difilter dari ' + allCount + ' total)';
        }}
        infoEl.textContent = text;
      }}

      const pagesContainer = document.getElementById('paginationPages');
      pagesContainer.innerHTML = '';

      const prevBtn = document.getElementById('prevPageBtn');
      const nextBtn = document.getElementById('nextPageBtn');
      const firstBtn = document.getElementById('firstPageBtn');
      const lastBtn = document.getElementById('lastPageBtn');

      if (firstBtn) firstBtn.disabled = currentPage <= 1;
      if (prevBtn) prevBtn.disabled = currentPage <= 1;
      if (nextBtn) nextBtn.disabled = currentPage >= totalPages;
      if (lastBtn) lastBtn.disabled = currentPage >= totalPages;

      let startPage = Math.max(1, currentPage - 2);
      let endPage = Math.min(totalPages, currentPage + 2);
      if (endPage - startPage < 4) {{
        if (startPage === 1) {{
          endPage = Math.min(totalPages, startPage + 4);
        }} else if (endPage === totalPages) {{
          startPage = Math.max(1, endPage - 4);
        }}
      }}

      for (let p = startPage; p <= endPage; p++) {{
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'page-btn' + (p === currentPage ? ' active' : '');
        btn.textContent = p;
        btn.onclick = () => applyFiltersAndPagination(p);
        pagesContainer.appendChild(btn);
      }}
    }}

    function handleRowCheckboxChange(cb, ev) {{
      if (ev) ev.stopPropagation();
      const row = cb.closest('tr');
      const addr = row.getAttribute('data-addr');
      const raw = row.getAttribute('data-raw');
      if (cb.checked) {{
        selectedAddrs.add(addr);
        selectedRaws.set(addr, raw);
        row.classList.add('selected-row');
      }} else {{
        selectedAddrs.delete(addr);
        selectedRaws.delete(addr);
        row.classList.remove('selected-row');
      }}
      updateSelectAllCheckboxState();
      updateSelectionUI();
    }}

    function toggleSelectAll(masterCb) {{
      const filtered = getFilteredRows();
      const startIdx = (currentPage - 1) * pageSize;
      const endIdx = Math.min(startIdx + pageSize, filtered.length);
      const pageRows = filtered.slice(startIdx, endIdx);

      const shouldCheck = masterCb.checked;
      pageRows.forEach(row => {{
        const cb = row.querySelector('.row-checkbox');
        const addr = row.getAttribute('data-addr');
        const raw = row.getAttribute('data-raw');
        cb.checked = shouldCheck;
        if (shouldCheck) {{
          selectedAddrs.add(addr);
          selectedRaws.set(addr, raw);
          row.classList.add('selected-row');
        }} else {{
          selectedAddrs.delete(addr);
          selectedRaws.delete(addr);
          row.classList.remove('selected-row');
        }}
      }});
      updateSelectionUI();
    }}

    function updateSelectAllCheckboxState(pageRows) {{
      const masterCb = document.getElementById('selectAllCheckbox');
      if (!masterCb) return;
      if (!pageRows) {{
        const filtered = getFilteredRows();
        const startIdx = (currentPage - 1) * pageSize;
        const endIdx = Math.min(startIdx + pageSize, filtered.length);
        pageRows = filtered.slice(startIdx, endIdx);
      }}
      if (pageRows.length === 0) {{
        masterCb.checked = false;
        masterCb.indeterminate = false;
        return;
      }}
      let checkedCount = 0;
      pageRows.forEach(r => {{
        const cb = r.querySelector('.row-checkbox');
        if (cb && cb.checked) checkedCount++;
      }});
      if (checkedCount === 0) {{
        masterCb.checked = false;
        masterCb.indeterminate = false;
      }} else if (checkedCount === pageRows.length) {{
        masterCb.checked = true;
        masterCb.indeterminate = false;
      }} else {{
        masterCb.checked = false;
        masterCb.indeterminate = true;
      }}
    }}

    function updateSelectionUI() {{
      const count = selectedAddrs.size;
      const bar = document.getElementById('selectionBar');
      const countBadge = document.getElementById('selectedCountText');
      if (count > 0) {{
        bar.classList.add('active');
        countBadge.textContent = count + ' dipilih';
      }} else {{
        bar.classList.remove('active');
      }}
    }}

    function selectAllFiltered() {{
      const filtered = getFilteredRows();
      filtered.forEach(row => {{
        const cb = row.querySelector('.row-checkbox');
        const addr = row.getAttribute('data-addr');
        const raw = row.getAttribute('data-raw');
        if (cb) cb.checked = true;
        selectedAddrs.add(addr);
        selectedRaws.set(addr, raw);
        row.classList.add('selected-row');
      }});
      updateSelectAllCheckboxState();
      updateSelectionUI();
      showToast(filtered.length + ' proxy hasil filter dipilih');
    }}

    function clearAllSelection() {{
      selectedAddrs.clear();
      selectedRaws.clear();
      document.querySelectorAll('.row-checkbox').forEach(cb => cb.checked = false);
      document.querySelectorAll('.proxy-row').forEach(row => row.classList.remove('selected-row'));
      updateSelectAllCheckboxState();
      updateSelectionUI();
    }}

    function copySelectedProxies() {{
      if (selectedRaws.size === 0) return;
      const list = Array.from(selectedRaws.values());
      navigator.clipboard.writeText(list.join('\n')).then(() => {{
        showToast(list.length + ' proxy berhasil disalin ke clipboard');
      }}).catch(() => {{
        alert('Gagal menyalin proxy');
      }});
    }}

    function exportSelectedProxies() {{
      if (selectedRaws.size === 0) return;
      const list = Array.from(selectedRaws.values());
      const blob = new Blob([list.join('\n')], {{ type: 'text/plain' }});
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'selected_proxies.txt';
      a.click();
      URL.revokeObjectURL(url);
      showToast(list.length + ' proxy diexport ke file');
    }}

    function deleteSelectedProxies() {{
      const count = selectedAddrs.size;
      if (count === 0) return;
      if (!confirm('Hapus ' + count + ' proxy yang dipilih dari daftar?')) return;
      const textarea = document.getElementById('rawProxies');
      const lines = textarea.value.split('\n');
      const toDelete = new Set(selectedAddrs);
      const remaining = [];
      for (const line of lines) {{
        const trimmed = line.trim();
        if (trimmed.length === 0) {{
          remaining.push(line);
          continue;
        }}
        let matched = false;
        for (const addr of toDelete) {{
          if (trimmed.includes(addr)) {{
            matched = true;
            toDelete.delete(addr);
            break;
          }}
        }}
        if (!matched) {{
          remaining.push(line);
        }}
      }}
      textarea.value = remaining.join('\n');
      document.getElementById('proxyForm').submit();
    }}

    function copyRowProxy(btn) {{
      const row = btn.closest('tr');
      const raw = row.getAttribute('data-raw');
      navigator.clipboard.writeText(raw).then(() => {{
        showToast('Proxy disalin: ' + raw);
      }}).catch(() => {{
        alert('Gagal menyalin');
      }});
    }}

    function deleteRowProxy(btn) {{
      const row = btn.closest('tr');
      const addr = row.getAttribute('data-addr');
      deleteProxyByAddress(addr);
    }}

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

    function setupRowClicks() {{
      document.querySelectorAll('.proxy-row').forEach(row => {{
        row.addEventListener('click', (e) => {{
          if (['INPUT', 'BUTTON', 'A', 'CODE'].includes(e.target.tagName)) return;
          const cb = row.querySelector('.row-checkbox');
          if (cb) {{
            cb.checked = !cb.checked;
            handleRowCheckboxChange(cb);
          }}
        }});
      }});
    }}

    window.addEventListener('DOMContentLoaded', () => {{
      setupRowClicks();
      applyFiltersAndPagination(1);
    }});
    if (document.readyState === 'complete' || document.readyState === 'interactive') {{
      setupRowClicks();
      applyFiltersAndPagination(1);
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
