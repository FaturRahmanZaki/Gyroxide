# Gyroxide 🌀

**Gyroxide** is a high-performance auto-rotating proxy gateway built with async Rust (`Tokio` + `Hyper`). It acts as a single entry point (1 URL endpoint) that distributes incoming requests across bulk upstream proxies (HTTP, HTTPS, SOCKS5) using **Smart Round Robin**, dead proxy auto-skipping, background health-checking, a **Mini Web Dashboard UI**, and native **Prometheus & Grafana** metrics monitoring.

---

## Key Features

- **Single Gateway Endpoint**: Route all client traffic through a single entry point (`http://localhost:8080`).
- **Multi-Protocol Upstream**: Supports **HTTP**, **HTTPS**, and **SOCKS5** proxies (with or without username/password authentication).
- **Auto-Parsing Format**: Seamlessly supports standard proxy provider formats like `ip:port:user:pass` without manual conversion.
- **Smart Round Robin**: Intelligently rotates proxies per request and skips dead/unreachable proxies.
- **Unlimited Failover Retry**: Automatically tries the next proxy in the list upon failure or timeout until a working proxy is found.
- **Background Health Check & Auto Recovery**: Continuously probes dead proxies in the background; revived proxies rejoin rotation automatically.
- **HTTPS Tunneling (CONNECT)**: Full transparent HTTP `CONNECT` method support for TLS/SSL HTTPS traffic.
- **Mini Web Dashboard UI**: Edit, paste, and reload proxy lists directly from your browser (`/admin`) with secure Basic Authentication.
- **Prometheus & Grafana Ready**: Built-in `/metrics` endpoint to monitor traffic, response status codes, latencies, and proxy health in real-time.
- **Hardened Security**: Features Proxy Authentication, Dashboard Auth, and Anti-SSRF protection (blocks requests targeting private/internal network IPs).

---

## Project Structure

```text
Gyroxide/
├── src/
│   ├── main.rs           # Server entry point & connection listener
│   ├── config.rs         # Environment configuration parser
│   ├── proxy_manager.rs  # Rotation logic, health-check, & proxy list manager
│   ├── handler.rs        # HTTP reverse forwarding & CONNECT tunneling
│   ├── metrics.rs        # Prometheus exporter & Mini Web Dashboard
│   └── logger.rs         # Tracing subscriber & daily rolling file logger
├── Dockerfile            # Multi-stage production Dockerfile
├── docker-compose.yml    # Docker service definitions
├── proxies.txt           # Upstream proxy list
├── .env                  # Environment configuration
├── .env.example          # Environment template
└── Cargo.toml            # Rust manifest and dependencies
```

---

## Proxy List Format (`proxies.txt`)

Place one proxy per line. Lines starting with `#` or empty lines are ignored automatically.

```text
# Standard provider format:
45.38.107.97:6014:piholwfu:9bjw4d9t5t6e

# HTTP without auth:
1.2.3.4:8080
http://1.2.3.4:8080

# HTTP with auth:
http://username:password@1.2.3.4:8080

# HTTPS Proxy:
https://1.2.3.4:8443

# SOCKS5 without auth:
socks5://5.6.7.8:1080

# SOCKS5 with auth:
socks5://username:password@5.6.7.8:1080
```

---

## Configuration (`.env`)

Copy from `.env.example`:

```bash
cp .env.example .env
```

Configuration parameters:
```env
# Gateway proxy port
PORT=8080

# Metrics exporter & Web Dashboard port
METRICS_PORT=49152

# Upstream proxy file path
PROXY_FILE=proxies.txt

# Log file path
LOG_FILE=logs/proxy.log

# Authentication credentials (Recommended for VPS deployment!)
# Protects both the proxy gateway and the Web Dashboard
AUTH_USERNAME=admin
AUTH_PASSWORD=your_secure_password

# Request timeout in seconds
REQUEST_TIMEOUT_SECS=3

# Background health-check interval in seconds
HEALTH_CHECK_INTERVAL_SECS=60

# Number of consecutive failures before marking a proxy dead
MAX_FAILURES=3

# Block requests targeting private/internal IP addresses (Anti-SSRF)
BLOCK_PRIVATE_IPS=true
```

---

## 🖥️ Mini Web Dashboard (`/admin`)

Gyroxide includes a modern web dashboard for managing proxies without SSH terminal access.

- **URL (Docker Compose)**: `http://localhost:49153/admin` (or `http://YOUR_VPS_IP:49153/admin`)
- **URL (Native / Binary)**: `http://localhost:49152/admin`
- Enter the **Username** and **Password** configured in your `.env`.

Dashboard Highlights:
- **Real-time Stats**: View total, active (alive), and dead proxies.
- **Search, Filter & Pagination**: Filter by protocol, status, or search query with configurable page sizes.
- **Quick Add & Bulk Upload**: Add proxies individually or paste bulk proxy lists with automatic deduplication.
- **Raw Editor**: View and update the underlying `proxies.txt` file directly.
- **Batch Actions**: Select multiple proxies to copy, export to `.txt`, or batch delete.
- **Instant Save & Reload**: Updates the proxy list in-memory immediately without restarting the server!

---

## 📈 Monitoring (Prometheus & Grafana)

Prometheus metrics endpoint is available at:
- **Docker Compose**: `http://localhost:49153/metrics`
- **Native / Binary**: `http://localhost:49152/metrics`

### 1. Prometheus Scrape Configuration (`prometheus.yml`):
```yaml
scrape_configs:
  - job_name: 'gyroxide'
    scrape_interval: 5s
    static_configs:
      - targets: ['localhost:49153'] # or localhost:49152 if run natively
```

### 2. Available Metrics & PromQL Examples:

| Metric | PromQL Query | Visualization |
| :--- | :--- | :--- |
| **Total Proxies** | `gyroxide_proxies_total` | Stat Card |
| **Active Proxies** | `gyroxide_proxies_alive` | Stat Card (Green) |
| **Dead Proxies** | `gyroxide_proxies_dead` | Stat Card (Red) |
| **Request Rate (RPS)** | `sum(rate(gyroxide_requests_total[1m]))` | Time-series Graph |
| **Status Codes** | `sum by (status) (rate(gyroxide_requests_total[5m]))` | Pie / Bar Chart |
| **Average Latency** | `rate(gyroxide_request_duration_seconds_sum[1m]) / rate(gyroxide_request_duration_seconds_count[1m])` | Time-series (seconds) |
| **Top Failed Proxies** | `topk(5, gyroxide_proxy_failures_total)` | Bar Gauge / Table |

---

## 🛡️ VPS Security & Hardening

When deploying Gyroxide to a public VPS:

1. **Always Set Credentials**:
   Always set strong `AUTH_USERNAME` and `AUTH_PASSWORD` values in `.env`. Unauthenticated proxy requests are rejected with `407 Proxy Authentication Required`, and dashboard requests require `401 Unauthorized` Basic Auth.
2. **Change Default Ports**:
   Change default ports (`PORT=8080`, `METRICS_PORT=49152`, host port `49153`) in `.env` and `docker-compose.yml` to minimize exposure to public port scanners.
3. **Anti-SSRF Protection**:
   Keep `BLOCK_PRIVATE_IPS=true` enabled to prevent attackers from using the proxy to scan internal server services (`127.0.0.1`, `localhost`, cloud metadata `169.254.169.254`).

---

## Deployment & Running

### Using Docker Compose (Recommended):

Build the image:
```bash
docker compose build
```

Run in the background:
```bash
docker compose up -d
```

View real-time logs:
```bash
docker compose logs -f
```

Stop the service:
```bash
docker compose down
```

---

## Usage Examples

Configure your tools or scripts to use Gyroxide as an HTTP/HTTPS proxy:

### 1. cURL (Terminal)
```bash
# Without authentication:
curl.exe -x http://localhost:8080 https://httpbin.org/ip

# With authentication:
curl.exe -x http://admin:your_secure_password@localhost:8080 https://httpbin.org/ip
```

### 2. Python (Requests)
```python
import requests

proxies = {
    "http": "http://admin:your_secure_password@localhost:8080",
    "https": "http://admin:your_secure_password@localhost:8080",
}

response = requests.get("https://httpbin.org/ip", proxies=proxies)
print(response.json())
```

### 3. Node.js (Axios)
```javascript
const axios = require('axios');

axios.get('https://httpbin.org/ip', {
    proxy: {
        protocol: 'http',
        host: 'localhost',
        port: 8080,
        auth: {
            username: 'admin',
            password: 'your_secure_password'
        }
    }
}).then(res => console.log(res.data));
```

---

## License

This project is licensed under the **[PolyForm Noncommercial License 1.0.0](LICENSE)**.

- **Permitted**: Use for personal, educational, research, or hobby purposes. Forking and modifying the code is fully allowed under noncommercial terms.
- **Prohibited**: Commercial use, monetization, paid services, or revenue-generating business operations.

For official terms and details, visit [polyformproject.org](https://polyformproject.org/licenses/noncommercial/1.0.0).
