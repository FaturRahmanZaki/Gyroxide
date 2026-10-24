use std::{net::SocketAddr, sync::Arc};
use anyhow::Result;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tracing::info;

mod config;
mod handler;
mod logger;
mod metrics;
mod proxy_manager;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Arc::new(config::Config::from_env()?);

    logger::setup_logger(&config.log_file)?;

    let proxy_manager = Arc::new(
        proxy_manager::ProxyManager::new(&config.proxy_file, config.clone()).await?,
    );

    let metrics = Arc::new(metrics::Metrics::new()?);
    let entries = proxy_manager.get_proxy_entries().await;
    let (alive, total) = proxy_manager.get_stats().await;
    metrics.update_proxy_stats(&entries);

    {
        let pm = proxy_manager.clone();
        tokio::spawn(async move {
            pm.start_health_check().await;
        });
    }

    {
        let pm = proxy_manager.clone();
        let m = metrics.clone();
        let metrics_port = config.metrics_port;
        tokio::spawn(async move {
            if let Err(e) = metrics::start_metrics_server(metrics_port, pm, m).await {
                tracing::error!("Metrics server error: {}", e);
            }
        });
    }

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!("Gyroxide proxy gateway running on {}", addr);
    info!("Proxies loaded: {}/{} alive", alive, total);
    info!("Health check every {}s", config.health_check_interval_secs);

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let pm = proxy_manager.clone();
        let m = metrics.clone();

        tokio::spawn(async move {
            if let Err(e) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let pm = pm.clone();
                        let m = m.clone();
                        async move { handler::handle_request(req, pm, m, peer_addr).await }
                    }),
                )
                .with_upgrades()
                .await
            {
                tracing::debug!("Connection closed: {}", e);
            }
        });
    }
}
