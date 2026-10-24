# Gyroxide 🌀

**Gyroxide** adalah high-performance auto-rotating proxy gateway berbasis Rust async (`Tokio` + `Hyper`). Gyroxide bertindak sebagai satu pintu gerbang tunggal (1 URL endpoint) yang mendistribusikan request ke ratusan upstream proxy (HTTP, HTTPS, SOCKS5) secara bergilir dengan mekanisme *Smart Round Robin*, auto-skip proxy mati, automatic background health-checking, **Mini Web Dashboard UI**, dan integrasi monitoring **Prometheus & Grafana**.

---

## Fitur Utama

- **Single Gateway Endpoint**: Mengarahkan seluruh traffic client ke 1 URL (`http://localhost:8080`).
- **Multi-Protocol Upstream**: Mendukung upstream proxy jenis **HTTP**, **HTTPS**, dan **SOCKS5** (dengan atau tanpa username/password auth).
- **Format Otomatis**: Mendukung langsung format provider seperti `ip:port:user:pass` tanpa perlu convert manual.
- **Smart Round Robin**: Rotasi request otomatis bergilir dan cerdas (otomatis melewati proxy yang berstatus dead/down).
- **Unlimited Failover Retry**: Jika proxy gagal atau timeout, otomatis mencoba proxy berikutnya sampai menemukan proxy yang aktif.
- **Background Health Check & Auto Recovery**: Melakukan probe berkala ke proxy mati di background; proxy yang kembali online akan otomatis masuk ke rotasi lagi.
- **HTTPS Tunneling (CONNECT)**: Mendukung metode HTTP `CONNECT` untuk request HTTPS / SSL secara transparan.
- **Mini Web Dashboard UI**: Edit, paste, dan reload list proxy langsung dari browser (`/admin`) dengan autentikasi aman tanpa perlu sentuh terminal/SSH.
- **Prometheus & Grafana Ready**: Menyediakan endpoint `/metrics` bawaan untuk monitoring traffic, latency, dan kesehatan proxy secara live.
- **Keamanan Tingkat Tinggi**: Dilengkapi Proxy Authentication, Dashboard Auth, dan proteksi Anti-SSRF (pemblokiran akses ke IP internal/private VPS).

---

## Struktur Direktori

```text
Gyroxide/
├── src/
│   ├── main.rs           # Entry point server & socket listener
│   ├── config.rs         # Model konfigurasi membaca .env
│   ├── proxy_manager.rs  # Logika rotasi, status tracking, & background health-check
│   ├── handler.rs        # HTTP reverse forwarding & CONNECT tunneling
│   ├── metrics.rs        # Prometheus metrics exporter & Mini Web Dashboard
│   └── logger.rs         # Logging subscriber & daily rolling file appender
├── Dockerfile            # Multi-stage production Dockerfile
├── docker-compose.yml    # Service definition
├── proxies.txt           # File daftar upstream proxy
├── .env                  # Konfigurasi environment lokal/VPS
├── .env.example          # Contoh template konfigurasi
└── Cargo.toml            # Manifest dependensi Rust
```

---

## Format List Proxy (`proxies.txt`)

Tuliskan 1 proxy per baris. Baris yang diawali tanda `#` atau baris kosong akan diabaikan secara otomatis.

```text
# Format standar provider proxy:
45.38.107.97:6014:piholwfu:9bjw4d9t5t6e

# HTTP tanpa autentikasi:
1.2.3.4:8080
http://1.2.3.4:8080

# HTTP dengan autentikasi:
http://username:password@1.2.3.4:8080

# HTTPS Proxy:
https://1.2.3.4:8443

# SOCKS5 tanpa autentikasi:
socks5://5.6.7.8:1080

# SOCKS5 dengan autentikasi:
socks5://username:password@5.6.7.8:1080
```

---

## Konfigurasi (`.env`)

Cukup salin dari `.env.example`:

```bash
cp .env.example .env
```

Contoh konfigurasi `.env`:
```env
# Port gateway proxy
PORT=8080

# Port metrics exporter & Web Dashboard
METRICS_PORT=9090

# File upstream proxies (1 baris 1 proxy)
PROXY_FILE=proxies.txt

# File log
LOG_FILE=logs/proxy.log

# Kredensial autentikasi (Wajib diisi saat deploy ke VPS!)
# Digunakan untuk auth Proxy Gateway dan Login Web Dashboard
AUTH_USERNAME=admin
AUTH_PASSWORD=your_secure_password

# Timeout per request (detik)
REQUEST_TIMEOUT_SECS=15

# Interval pengecekan proxy mati di background (detik)
HEALTH_CHECK_INTERVAL_SECS=60

# Berapa kali gagal berturut-turut sebelum proxy ditandai dead
MAX_FAILURES=3

# Blokir request yang mengarah ke jaringan lokal / private VPS (Anti-SSRF)
BLOCK_PRIVATE_IPS=true
```

---

## 🖥️ Mini Web Dashboard (`/admin`)

Gyroxide dilengkapi dashboard web modern untuk mengelola proxy tanpa perlu buka SSH terminal.

1. Buka browser: `http://localhost:9090/admin` (atau `http://IP_VPS:9090/admin`)
2. Masukkan **Username** dan **Password** sesuai `AUTH_USERNAME` dan `AUTH_PASSWORD` di `.env`.
3. Fitur Dashboard:
   - **Statistik Realtime**: Total proxy, jumlah proxy aktif (*alive*), dan proxy mati (*dead*).
   - **Live Editor (Textarea)**: Paste atau hapus bulk proxy kapan saja.
   - **Simpan & Reload**: Klik tombol simpan, sistem langsung memperbarui list proxy di memory secara instan tanpa perlu restart server!

---

## 📈 Monitoring (Prometheus & Grafana)

Endpoint metrics standar Prometheus tersedia di:
`http://localhost:9090/metrics`

### 1. Tambahkan Scrape Job di `prometheus.yml`:
```yaml
scrape_configs:
  - job_name: 'gyroxide'
    scrape_interval: 5s
    static_configs:
      - targets: ['localhost:9090'] # atau IP_VPS:9090
```

### 2. Metrik yang Disediakan & Contoh PromQL Grafana:

| Metrik | PromQL Query | Visualisasi |
| :--- | :--- | :--- |
| **Total Proxies** | `gyroxide_proxies_total` | Stat Card |
| **Active Proxies** | `gyroxide_proxies_alive` | Stat Card (Hijau) |
| **Dead Proxies** | `gyroxide_proxies_dead` | Stat Card (Merah) |
| **Request Rate (RPS)** | `sum(rate(gyroxide_requests_total[1m]))` | Time-series Graph |
| **Status Codes** | `sum by (status) (rate(gyroxide_requests_total[5m]))` | Pie / Bar Chart |
| **Average Latency** | `rate(gyroxide_request_duration_seconds_sum[1m]) / rate(gyroxide_request_duration_seconds_count[1m])` | Time-series (detik) |
| **Top Gagal** | `topk(5, gyroxide_proxy_failures_total)` | Bar Gauge / Table |

---

## 🛡️ Panduan Keamanan VPS (Anti-Hacking)

Saat menjalankan Gyroxide di VPS Linux publik:

1. **Wajib Pasang Kredensial**:
   Selalu isi `AUTH_USERNAME` dan `AUTH_PASSWORD` yang kuat di `.env`. Permintaan proxy tanpa autentikasi akan ditolak dengan error `407 Proxy Authentication Required`, dan akses Web Dashboard akan ditolak dengan `401 Unauthorized`.
2. **Ganti Port Default**:
   Ganti port default (`PORT=8080`, `METRICS_PORT=9090`) ke port acak di `.env` dan `docker-compose.yml` agar tidak mudah terdeteksi bot scanner publik.
3. **Anti-SSRF**:
   Tetap aktifkan `BLOCK_PRIVATE_IPS=true` agar tidak ada pihak luar yang dapat memanfaatkan proxy untuk membobol port lokal VPS (`127.0.0.1`, `localhost`, metadata cloud `169.254.169.254`).

---

## Cara Menjalankan

### Menggunakan Docker Compose (Rekomendasi Production):

Build image:
```bash
docker compose build
```

Jalankan container di background:
```bash
docker compose up -d
```

Melihat stream log:
```bash
docker compose logs -f
```

Menghentikan container:
```bash
docker compose down
```

---

## Contoh Pemakaian di Client

Gunakan URL gateway Gyroxide sebagai proxy HTTP/HTTPS:

### 1. cURL (Terminal)
```bash
# Tanpa autentikasi (jika AUTH_USERNAME kosong):
curl.exe -x http://localhost:8080 https://httpbin.org/ip

# Dengan autentikasi:
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

## Lisensi

Proyek ini dilisensikan di bawah lisensi **[PolyForm Noncommercial License 1.0.0](LICENSE)**.

- **Boleh**: Digunakan untuk kebutuhan pribadi, riset, belajar, edukasi, hobi, dan di-fork/dimodifikasi secara bebas selama bersifat non-komersial.
- **Tidak Boleh**: Digunakan untuk tujuan komersial, monetisasi, produk berbayar, atau aktivitas bisnis yang menghasilkan pendapatan komersial/finansial.

Informasi teks resmi dan legalitas: [polyformproject.org](https://polyformproject.org/licenses/noncommercial/1.0.0).
