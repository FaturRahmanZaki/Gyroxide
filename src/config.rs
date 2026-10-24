use anyhow::Result;
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub metrics_port: u16,
    pub proxy_file: String,
    pub log_file: String,
    pub health_check_interval_secs: u64,
    pub health_check_url: String,
    pub request_timeout_secs: u64,
    pub max_failures: u32,
    pub auth_username: Option<String>,
    pub auth_password: Option<String>,
    pub block_private_ips: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .unwrap_or(8080);

        let metrics_port = env::var("METRICS_PORT")
            .unwrap_or_else(|_| "9090".to_string())
            .parse::<u16>()
            .unwrap_or(9090);

        let proxy_file = env::var("PROXY_FILE")
            .unwrap_or_else(|_| "proxies.txt".to_string());

        let log_file = env::var("LOG_FILE")
            .unwrap_or_else(|_| "logs/proxy.log".to_string());

        let health_check_interval_secs = env::var("HEALTH_CHECK_INTERVAL_SECS")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .unwrap_or(60);

        let health_check_url = env::var("HEALTH_CHECK_URL")
            .unwrap_or_else(|_| "http://httpbin.org/ip".to_string());

        let request_timeout_secs = env::var("REQUEST_TIMEOUT_SECS")
            .unwrap_or_else(|_| "15".to_string())
            .parse::<u64>()
            .unwrap_or(15);

        let max_failures = env::var("MAX_FAILURES")
            .unwrap_or_else(|_| "3".to_string())
            .parse::<u32>()
            .unwrap_or(3);

        let auth_username = env::var("AUTH_USERNAME")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() { None } else { Some(trimmed) }
            });

        let auth_password = env::var("AUTH_PASSWORD")
            .ok()
            .and_then(|v| {
                let trimmed = v.trim().to_string();
                if trimmed.is_empty() { None } else { Some(trimmed) }
            });

        let block_private_ips = env::var("BLOCK_PRIVATE_IPS")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        Ok(Self {
            port,
            metrics_port,
            proxy_file,
            log_file,
            health_check_interval_secs,
            health_check_url,
            request_timeout_secs,
            max_failures,
            auth_username,
            auth_password,
            block_private_ips,
        })
    }
}
