use anyhow::{anyhow, Result};
use base64::Engine;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::{net::SocketAddr, sync::Arc, time::{Duration, Instant}};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::{error, info, warn};

use crate::metrics::Metrics;
use crate::proxy_manager::{ProxyEntry, ProxyManager, ProxyType};

pub type BoxResponse = Response<BoxBody<Bytes, hyper::Error>>;

pub async fn handle_request(
    req: Request<Incoming>,
    proxy_manager: Arc<ProxyManager>,
    metrics: Arc<Metrics>,
    peer_addr: SocketAddr,
) -> Result<BoxResponse> {
    let start = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();
    let timeout = Duration::from_secs(proxy_manager.config.request_timeout_secs);
    let total_proxies = proxy_manager.total_count().await;
    let max_attempts = if total_proxies > 0 { total_proxies } else { 1 };

    let config = &proxy_manager.config;

    if let (Some(expected_user), Some(expected_pass)) = (&config.auth_username, &config.auth_password) {
        if !validate_proxy_auth(req.headers(), expected_user, expected_pass) {
            metrics
                .requests_total
                .with_label_values(&[method.as_str(), "407"])
                .inc();
            warn!(client = %peer_addr, "Proxy authentication required/failed");
            return Ok(proxy_auth_required_response());
        }
    }

    if method == Method::CONNECT {
        let authority = req.uri().authority().map(|a| a.as_str()).unwrap_or("");
        if config.block_private_ips && is_private_target(authority) {
            metrics
                .requests_total
                .with_label_values(&["CONNECT", "403"])
                .inc();
            warn!(client = %peer_addr, target = %authority, "Blocked request to private/internal network");
            return Ok(error_response(StatusCode::FORBIDDEN, "Forbidden: private network access blocked"));
        }
        return handle_connect(req, proxy_manager, metrics, peer_addr, timeout, max_attempts, start).await;
    }

    if config.block_private_ips {
        if let Some(host) = req.uri().host() {
            if is_private_target(host) {
                metrics
                    .requests_total
                    .with_label_values(&[method.as_str(), "403"])
                    .inc();
                warn!(client = %peer_addr, host = %host, "Blocked request to private/internal network");
                return Ok(error_response(StatusCode::FORBIDDEN, "Forbidden: private network access blocked"));
            }
        }
    }

    let (parts, body) = req.into_parts();
    let body_bytes = match body.collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return Ok(error_response(StatusCode::BAD_REQUEST, &e.to_string())),
    };

    let mut last_err = String::from("no proxies available");

    for attempt in 0..max_attempts {
        let proxy = match proxy_manager.next_proxy().await {
            Some(p) => p,
            None => break,
        };
        let proxy_url = proxy.url.clone();

        match forward_http(&parts, body_bytes.clone(), &proxy, timeout).await {
            Ok(response) => {
                let status_str = response.status().as_u16().to_string();
                let elapsed_sec = start.elapsed().as_secs_f64();
                metrics
                    .requests_total
                    .with_label_values(&[parts.method.as_str(), &status_str])
                    .inc();
                metrics.request_duration_seconds.observe(elapsed_sec);

                info!(
                    client = %peer_addr,
                    method = %parts.method,
                    uri = %uri,
                    proxy = %proxy_url,
                    status = response.status().as_u16(),
                    ms = start.elapsed().as_millis(),
                    "request ok"
                );
                return Ok(response);
            }
            Err(e) => {
                metrics
                    .proxy_failures_total
                    .with_label_values(&[&proxy_url])
                    .inc();
                last_err = e.to_string();
                warn!(
                    attempt = attempt + 1,
                    max = max_attempts,
                    proxy = %proxy_url,
                    err = %e,
                    "proxy failed"
                );
                proxy_manager.mark_dead(&proxy_url).await;
            }
        }
    }

    metrics
        .requests_total
        .with_label_values(&[parts.method.as_str(), "502"])
        .inc();
    error!(client = %peer_addr, uri = %uri, err = %last_err, "all proxies failed");
    Ok(error_response(StatusCode::BAD_GATEWAY, "All proxies failed"))
}

async fn handle_connect(
    req: Request<Incoming>,
    proxy_manager: Arc<ProxyManager>,
    metrics: Arc<Metrics>,
    peer_addr: SocketAddr,
    timeout: Duration,
    max_attempts: usize,
    start: Instant,
) -> Result<BoxResponse> {
    let host_port = req
        .uri()
        .authority()
        .map(|a| a.to_string())
        .ok_or_else(|| anyhow!("CONNECT missing target"))?;

    let mut last_err = String::from("no proxies available");

    for attempt in 0..max_attempts {
        let proxy = match proxy_manager.next_proxy().await {
            Some(p) => p,
            None => break,
        };
        let proxy_url = proxy.url.clone();

        match connect_upstream(&proxy, &host_port, timeout).await {
            Ok(upstream) => {
                let elapsed_sec = start.elapsed().as_secs_f64();
                metrics
                    .requests_total
                    .with_label_values(&["CONNECT", "200"])
                    .inc();
                metrics.request_duration_seconds.observe(elapsed_sec);

                info!(
                    client = %peer_addr,
                    target = %host_port,
                    proxy = %proxy_url,
                    "CONNECT tunnel established"
                );

                tokio::spawn(async move {
                    match hyper::upgrade::on(req).await {
                        Ok(upgraded) => {
                            let mut client_io = TokioIo::new(upgraded);
                            let mut upstream_io = upstream;
                            if let Err(e) =
                                tokio::io::copy_bidirectional(&mut client_io, &mut upstream_io)
                                    .await
                            {
                                if e.kind() != std::io::ErrorKind::ConnectionReset {
                                    tracing::debug!("tunnel closed: {}", e);
                                }
                            }
                        }
                        Err(e) => error!("upgrade failed: {}", e),
                    }
                });

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .body(empty_body())?);
            }
            Err(e) => {
                metrics
                    .proxy_failures_total
                    .with_label_values(&[&proxy_url])
                    .inc();
                last_err = e.to_string();
                warn!(
                    attempt = attempt + 1,
                    max = max_attempts,
                    proxy = %proxy_url,
                    err = %e,
                    "CONNECT failed"
                );
                proxy_manager.mark_dead(&proxy_url).await;
            }
        }
    }

    metrics
        .requests_total
        .with_label_values(&["CONNECT", "502"])
        .inc();

    Ok(error_response(
        StatusCode::BAD_GATEWAY,
        &format!("CONNECT failed: {}", last_err),
    ))
}

async fn connect_upstream(proxy: &ProxyEntry, target: &str, timeout: Duration) -> Result<TcpStream> {
    tokio::time::timeout(timeout, async {
        match proxy.proxy_type {
            ProxyType::Http => connect_via_http_proxy(proxy, target).await,
            ProxyType::Socks5 => connect_via_socks5(proxy, target).await,
        }
    })
    .await
    .map_err(|_| anyhow!("connection timed out"))?
}

async fn connect_via_http_proxy(proxy: &ProxyEntry, target: &str) -> Result<TcpStream> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|e| anyhow!("cannot reach proxy {}: {}", addr, e))?;

    let mut req = format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n"
    );

    if let (Some(user), Some(pass)) = (&proxy.username, &proxy.password) {
        let creds = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", user, pass));
        req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", creds));
    }

    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).await?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let resp = String::from_utf8_lossy(&buf[..n]);

    if resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200") {
        Ok(stream)
    } else {
        let line = resp.lines().next().unwrap_or("unknown").to_string();
        Err(anyhow!("HTTP proxy CONNECT rejected: {}", line))
    }
}

async fn connect_via_socks5(proxy: &ProxyEntry, target: &str) -> Result<TcpStream> {
    use tokio_socks::tcp::Socks5Stream;

    let (host, port) = split_host_port(target)?;
    let proxy_addr = format!("{}:{}", proxy.host, proxy.port);

    let stream = match (&proxy.username, &proxy.password) {
        (Some(user), Some(pass)) => {
            Socks5Stream::connect_with_password(
                proxy_addr.as_str(),
                (host.as_str(), port),
                user.as_str(),
                pass.as_str(),
            )
            .await
            .map_err(|e| anyhow!("SOCKS5 auth connect failed: {}", e))?
        }
        _ => Socks5Stream::connect(proxy_addr.as_str(), (host.as_str(), port))
            .await
            .map_err(|e| anyhow!("SOCKS5 connect failed: {}", e))?,
    };

    Ok(stream.into_inner())
}

async fn forward_http(
    parts: &hyper::http::request::Parts,
    body: Bytes,
    proxy: &ProxyEntry,
    timeout: Duration,
) -> Result<BoxResponse> {
    let reqwest_proxy = reqwest::Proxy::all(&proxy.url)
        .map_err(|e| anyhow!("invalid proxy URL '{}': {}", proxy.url, e))?;

    let client = reqwest::Client::builder()
        .proxy(reqwest_proxy)
        .timeout(timeout)
        .build()
        .map_err(|e| anyhow!("client build failed: {}", e))?;

    let uri = parts.uri.to_string();
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;

    let mut builder = client.request(method, &uri);

    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        builder = builder.header(name, value);
    }

    if !body.is_empty() {
        builder = builder.body(body);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| anyhow!("request error: {}", e))?;

    let status = resp.status();
    let headers = resp.headers().clone();
    let body_bytes = resp
        .bytes()
        .await
        .map_err(|e| anyhow!("response read error: {}", e))?;

    let mut rb = Response::builder().status(status.as_u16());
    for (name, value) in &headers {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        rb = rb.header(name, value);
    }

    Ok(rb.body(full_body(body_bytes))?)
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn split_host_port(host_port: &str) -> Result<(String, u16)> {
    let idx = host_port
        .rfind(':')
        .ok_or_else(|| anyhow!("no port in '{}'", host_port))?;
    let host = host_port[..idx]
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    let port: u16 = host_port[idx + 1..]
        .parse()
        .map_err(|_| anyhow!("invalid port in '{}'", host_port))?;
    Ok((host, port))
}

fn empty_body() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full_body(bytes: impl Into<Bytes>) -> BoxBody<Bytes, hyper::Error> {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

fn error_response(status: StatusCode, msg: &str) -> BoxResponse {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(full_body(msg.as_bytes().to_vec()))
        .unwrap()
}

fn proxy_auth_required_response() -> BoxResponse {
    Response::builder()
        .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
        .header("proxy-authenticate", "Basic realm=\"Gyroxide Proxy\"")
        .header("content-type", "text/plain")
        .body(full_body("407 Proxy Authentication Required\n"))
        .unwrap()
}

fn validate_proxy_auth(headers: &hyper::HeaderMap, user: &str, pass: &str) -> bool {
    let header_val = match headers.get("proxy-authorization") {
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

    u == user && p == pass
}

fn is_private_target(host_or_authority: &str) -> bool {
    let host = if let Ok((h, _)) = split_host_port(host_or_authority) {
        h
    } else {
        host_or_authority.to_string()
    };

    let host_clean = host.trim_matches(|c| c == '[' || c == ']');

    if host_clean.eq_ignore_ascii_case("localhost")
        || host_clean.ends_with(".localhost")
        || host_clean.ends_with(".local")
        || host_clean.ends_with(".internal")
    {
        return true;
    }

    if let Ok(ip) = host_clean.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_loopback()
                    || ipv4.is_private()
                    || ipv4.is_link_local()
                    || ipv4.is_broadcast()
                    || ipv4.octets() == [169, 254, 169, 254]
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback()
            }
        }
    } else {
        false
    }
}

