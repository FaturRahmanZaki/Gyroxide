//! Admin dashboard: server-rendered HTML built from static assets in this folder.
//!
//! Layout:
//!   template.html    markup shell with `{{...}}` placeholders
//!   dashboard.css    stylesheet, inlined into the response
//!   dashboard.js     client-side behaviour (table, theme, language), inlined

use crate::proxy_manager::{ProxyEntry, ProxyType};

const TEMPLATE: &str = include_str!("template.html");
const STYLES: &str = include_str!("dashboard.css");
const SCRIPT: &str = include_str!("dashboard.js");
const LOGIN_TEMPLATE: &str = include_str!("login.html");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Success,
    Info,
    Error,
}

impl AlertKind {
    fn class(self) -> &'static str {
        match self {
            AlertKind::Success => "success",
            AlertKind::Info => "info",
            AlertKind::Error => "error",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            AlertKind::Success => {
                r#"<svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="m5.25 8.25 2 2 3.5-4"/></svg>"#
            }
            AlertKind::Info => {
                r#"<svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="M8 7.25v3.5M8 5.25v.01"/></svg>"#
            }
            AlertKind::Error => {
                r#"<svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="m6 6 4 4M10 6l-4 4"/></svg>"#
            }
        }
    }
}

/// A flash message rendered above the stats.
///
/// `message` is the server-side (Indonesian) fallback so the page still reads
/// correctly without JavaScript. `key` + `args` let the client re-render the
/// same message in whichever language is active.
#[derive(Debug, Clone, Copy)]
pub struct Flash<'a> {
    pub kind: AlertKind,
    pub key: &'a str,
    pub args: &'a [&'a str],
    pub message: &'a str,
}

pub fn render(
    entries: &[ProxyEntry],
    proxies_txt: &str,
    alert: Option<Flash<'_>>,
    grafana_url: Option<&str>,
) -> String {
    let total = entries.len();
    let alive = entries.iter().filter(|p| p.is_alive).count();
    let dead = total.saturating_sub(alive);
    let availability = if total == 0 {
        "—".to_string()
    } else {
        format!("{:.1}%", alive as f64 / total as f64 * 100.0)
    };

    let alert_box = match alert {
        Some(flash) => {
            let mut arg_attrs = String::new();
            for (index, arg) in flash.args.iter().enumerate() {
                arg_attrs.push_str(&format!(
                    r#" data-alert-arg{index}="{value}""#,
                    index = index,
                    value = html_escape(arg),
                ));
            }
            format!(
                r#"<div class="banner {class}" role="status" data-alert-key="{key}"{arg_attrs}>{icon}<span>{message}</span></div>"#,
                class = flash.kind.class(),
                key = html_escape(flash.key),
                arg_attrs = arg_attrs,
                icon = flash.kind.icon(),
                message = html_escape(flash.message),
            )
        }
        None => String::new(),
    };

    let rows = build_rows(entries, proxies_txt);

    // Static assets first, then the dynamic values, so proxy text can never
    // introduce another placeholder.
    let styles = format!("<style>\n{}\n</style>", STYLES);
    TEMPLATE
        .replace("{{CSS}}", &styles)
        .replace("{{JS}}", SCRIPT)
        .replace("{{TOTAL}}", &total.to_string())
        .replace("{{ALIVE}}", &alive.to_string())
        .replace("{{DEAD}}", &dead.to_string())
        .replace("{{AVAILABILITY}}", &availability)
        .replace("{{PROXY_TEXT}}", &html_escape(proxies_txt))
        .replace("{{ALERT}}", &alert_box)
        .replace("{{ROWS}}", &rows)
        .replace("{{GRAFANA_URL}}", &html_escape(grafana_url.unwrap_or("")))
}

/// Renders the login page. `show_error` fills the reserved notice slot with
/// the "credentials rejected" message after a failed POST /admin/login.
pub fn render_login(show_error: bool) -> String {
    LOGIN_TEMPLATE
        .replace("{{ERROR_CLASS}}", if show_error { " error" } else { "" })
        .replace(
            "{{ERROR_TEXT}}",
            if show_error {
                r#"Credentials rejected. Check <code>AUTH_USERNAME</code> / <code>AUTH_PASSWORD</code> in your <code>.env</code>."#
            } else {
                ""
            },
        )
}

fn build_rows(entries: &[ProxyEntry], proxies_txt: &str) -> String {
    let lines: Vec<&str> = proxies_txt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();

    // Match each parsed entry back to the raw line it came from, so copy/delete
    // round-trip the exact text the user wrote (including credentials).
    let mut used = vec![false; lines.len()];
    let mut out = String::with_capacity(entries.len() * 480);

    for entry in entries {
        let host_port = format!("{}:{}", entry.host, entry.port);
        let addr = html_escape(&host_port);

        let mut raw = host_port.clone();
        for (idx, line) in lines.iter().enumerate() {
            if !used[idx]
                && line.contains(&entry.host)
                && line.contains(&entry.port.to_string())
            {
                used[idx] = true;
                raw = (*line).to_string();
                break;
            }
        }

        let (proto_label, proto_key) = match entry.proxy_type {
            ProxyType::Http => ("HTTP", "http"),
            ProxyType::Socks5 => ("SOCKS5", "socks5"),
        };

        let auth = match &entry.username {
            Some(user) => format!(
                r#"<span class="auth" title="{title}">{name}</span>"#,
                title = html_escape(user),
                name = html_escape(user),
            ),
            None => r#"<span class="muted">—</span>"#.to_string(),
        };

        let (status_label, status_key, status_class) = if entry.is_alive {
            ("Alive", "alive", "status-alive")
        } else {
            ("Dead", "dead", "status-dead")
        };

        let fail_class = if entry.fail_count > 0 {
            "fail-count has-fail"
        } else {
            "fail-count"
        };

        let user = entry.username.as_deref().unwrap_or("");

        out.push_str(&format!(
            r#"<tr class="proxy-row" data-addr="{addr}" data-proto="{proto}" data-status="{status}" data-raw="{raw}" data-user="{user}">
  <td class="col-check"><input type="checkbox" class="row-checkbox" value="{addr}" aria-label="Pilih {addr}" onchange="handleRowCheckboxChange(this)"></td>
  <td class="col-num">1</td>
  <td class="col-addr"><code>{addr}</code></td>
  <td><span class="chip">{proto_label}</span></td>
  <td>{auth}</td>
  <td><span class="status {status_class}"><span class="status-dot"></span><span class="status-text">{status_label}</span></span></td>
  <td class="col-fail"><span class="{fail_class}">{fail_count}</span></td>
  <td class="col-act">
    <button type="button" class="icon-btn" title="Salin proxy" aria-label="Salin {addr}" onclick="copyRowProxy(this)"><svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5v-2a1 1 0 0 0-1-1h-6a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2"/></svg></button>
    <button type="button" class="icon-btn danger" title="Hapus proxy" aria-label="Hapus {addr}" onclick="deleteRowProxy(this)"><svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" aria-hidden="true"><path d="M2.5 4.5h11M6 4.5V3h4v1.5M4 4.5l.5 8h7l.5-8M6.5 7v3.5M9.5 7v3.5"/></svg></button>
  </td>
</tr>"#,
            addr = addr,
            proto = proto_key,
            status = status_key,
            raw = html_escape(&raw),
            user = html_escape(user),
            proto_label = proto_label,
            auth = auth,
            status_class = status_class,
            status_label = status_label,
            fail_class = fail_class,
            fail_count = entry.fail_count,
        ));
    }

    out
}

pub fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
