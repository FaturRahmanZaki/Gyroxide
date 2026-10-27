use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use prometheus::{
    register_counter_vec, register_gauge, register_gauge_vec, register_histogram, CounterVec, Encoder,
    Gauge, GaugeVec, Histogram, TextEncoder,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::dashboard::{self, AlertKind, Flash};
use crate::proxy_manager::{ProxyEntry, ProxyManager, ProxyType};

const SESSION_COOKIE: &str = "gyroxide_session";
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// In-memory dashboard sessions. Logout works because the dashboard honors a
/// session cookie instead of raw Basic Auth — the browser would otherwise
/// silently resend its cached credentials forever.
struct SessionStore {
    sessions: Mutex<HashMap<String, Instant>>,
}

impl SessionStore {
    fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn issue(&self) -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();

        let mut sessions = self.sessions.lock().unwrap();
        let now = Instant::now();
        sessions.retain(|_, issued| now.duration_since(*issued) < SESSION_TTL);
        sessions.insert(token.clone(), now);
        token
    }

    fn validate(&self, token: &str) -> bool {
        let sessions = self.sessions.lock().unwrap();
        sessions
            .get(token)
            .map(|issued| Instant::now().duration_since(*issued) < SESSION_TTL)
            .unwrap_or(false)
    }

    fn revoke(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }
}

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
    let sessions = Arc::new(SessionStore::new());
    info!("Gyroxide Web Dashboard & Metrics running on http://{}/admin", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let pm = proxy_manager.clone();
        let m = metrics.clone();
        let sessions = sessions.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<Incoming>| {
                let pm = pm.clone();
                let m = m.clone();
                let sessions = sessions.clone();
                async move {
                    handle_web_request(req, pm, m, sessions).await
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
    sessions: Arc<SessionStore>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let query = req.uri().query().map(str::to_owned);
    let session_cookie = session_from_headers(req.headers());
    let config = pm.config.clone();

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

    let auth_enabled = config.auth_username.is_some() && config.auth_password.is_some();

    if auth_enabled {
        if (method == Method::GET || method == Method::HEAD) && path == "/login" {
            let show_error = query
                .as_deref()
                .map(|q| q.split('&').any(|kv| kv == "error=1"))
                .unwrap_or(false);
            return Ok(html_response(dashboard::render_login(show_error)));
        }

        if method == Method::POST && path == "/admin/login" {
            let body = match req.into_body().collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(e) => {
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Full::new(Bytes::from(format!("Body error: {}", e))))
                        .unwrap());
                }
            };
            let params = parse_form(&String::from_utf8_lossy(&body));
            let username = params.iter().find(|(k, _)| k == "username").map(|(_, v)| v.as_str());
            let password = params.iter().find(|(k, _)| k == "password").map(|(_, v)| v.as_str());

            let credentials_ok = matches!(
                (&config.auth_username, &config.auth_password, username, password),
                (Some(expected_u), Some(expected_p), Some(input_u), Some(input_p))
                    if input_u == expected_u && input_p == expected_p
            );

            if credentials_ok {
                let token = sessions.issue();
                info!("Dashboard login succeeded");
                return Ok(redirect(
                    "/admin",
                    Some(format!(
                        "{}={}; HttpOnly; SameSite=Lax; Path=/",
                        SESSION_COOKIE, token
                    )),
                ));
            }
            warn!("Dashboard login failed");
            return Ok(redirect("/login?error=1", None));
        }

        if method == Method::GET && path == "/admin/logout" {
            if let Some(token) = &session_cookie {
                sessions.revoke(token);
            }
            return Ok(redirect(
                "/login",
                Some(format!(
                    "{}=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0",
                    SESSION_COOKIE
                )),
            ));
        }

        let authenticated = session_cookie
            .as_deref()
            .map(|t| sessions.validate(t))
            .unwrap_or(false);
        if !authenticated {
            return Ok(redirect("/login", None));
        }
    }

    if (method == Method::GET || method == Method::HEAD) && (path == "/" || path == "/admin") {
        let entries = pm.get_proxy_entries().await;
        let proxy_text = pm.get_raw_proxy_text().await;
        let html = dashboard::render(&entries, &proxy_text, None, config.grafana_url.as_deref());

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
        let html = dashboard::render(&entries, &proxy_text, Some(flash), config.grafana_url.as_deref());

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

    for (key, val) in parse_form(input) {
        if key == "proxies" {
            proxies = Some(val);
        } else if key == "mode" {
            mode = Some(val);
        }
    }

    (proxies, mode)
}

/// Parses an `application/x-www-form-urlencoded` body into key/value pairs.
fn parse_form(input: &str) -> Vec<(String, String)> {
    if !input.contains('=') {
        return Vec::new();
    }

    input
        .split('&')
        .filter_map(|part| {
            let (key, val) = part.split_once('=')?;
            Some((urlencoding_decode(key), urlencoding_decode(val)))
        })
        .collect()
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

fn html_response(body: String) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn redirect(location: &str, set_cookie: Option<String>) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("Location", location);
    if let Some(cookie) = set_cookie {
        builder = builder.header("Set-Cookie", cookie);
    }
    builder
        .body(Full::new(Bytes::from(format!("Redirecting to {}", location))))
        .unwrap()
}

/// Extracts the session token from the request's `Cookie` header, if any.
fn session_from_headers(headers: &hyper::HeaderMap) -> Option<String> {
    let cookies = headers.get("cookie")?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        if name == SESSION_COOKIE {
            Some(value.to_string())
        } else {
            None
        }
    })
}


