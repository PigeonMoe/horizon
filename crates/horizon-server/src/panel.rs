//! The web panel (docs/server.md, section 6): one page embedded in the binary, served by the supervisor on
//! `[panel] bind` (127.0.0.1:8130 by default). It uses the control API's routes with a browser session.
//!
//! - **Login.** `horizon-server panel` asks the API for a link with a one-time code, good for 10 minutes;
//!   opening it swaps the code for an `HttpOnly; SameSite=Strict` session cookie. Anyone who can read the
//!   server folder's token can make a link. A configured HTTPS public origin uses a dedicated access key
//!   in exchange for a short-lived `Secure` session cookie. Sessions end when the supervisor does.
//! - **Rebinding and CSRF.** The `Host` header must name the panel's own address (`localhost` too on
//!   loopback, a MagicDNS name on a tailnet, any IP address when bound to every interface). Public mode
//!   accepts only the configured authority, and requires its HTTPS origin on changes. Every change needs
//!   the `X-Horizon: 1` header, which a foreign page can't send without the browser asking first.
//! - **The page** loads nothing from elsewhere (a strict Content-Security-Policy) and can't be framed.

use crate::{
    api::{self, Shared},
    http::{self, Request, Response},
};
use horizon_server_launch::config::PANEL_ACCESS_KEY_ENV;
use serde::Deserialize;
use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket},
    sync::Arc,
    time::{Duration, Instant},
};

mod plugins;

const COOKIE: &str = "horizon_panel";
const PUBLIC_COOKIE: &str = "__Host-horizon_panel";
/// How long a login link works.
const LINK_LIFE: Duration = Duration::from_secs(600);
const PUBLIC_SESSION_LIFE: Duration = Duration::from_secs(12 * 60 * 60);
/// Failed logins allowed per minute before the panel refuses them for a while.
const FAILURES_PER_MINUTE: usize = 10;
const SESSIONS: usize = 32;

const PAGE: &str = include_str!("../panel/index.html");
const SCRIPT: &str = include_str!("../panel/panel.js");
const ZH_CN: &str = include_str!("../panel/zh-cn.js");
const STYLE: &str = include_str!("../panel/panel.css");

/// The fonts and the mark the page uses, from horizon-theme (Geist and Bricolage Grotesque, SIL OFL).
const ASSETS: [(&str, &str, &[u8]); 6] = [
    (
        "/mark.png",
        "image/png",
        include_bytes!("../../horizon-theme/icons/horizon-64.png"),
    ),
    (
        "/fonts/Geist-Regular.ttf",
        "font/ttf",
        include_bytes!("../../horizon-theme/fonts/Geist-Regular.ttf"),
    ),
    (
        "/fonts/Geist-Medium.ttf",
        "font/ttf",
        include_bytes!("../../horizon-theme/fonts/Geist-Medium.ttf"),
    ),
    (
        "/fonts/Geist-SemiBold.ttf",
        "font/ttf",
        include_bytes!("../../horizon-theme/fonts/Geist-SemiBold.ttf"),
    ),
    (
        "/fonts/GeistMono-Regular.ttf",
        "font/ttf",
        include_bytes!("../../horizon-theme/fonts/GeistMono-Regular.ttf"),
    ),
    (
        "/fonts/BricolageGrotesqueHeading-SemiBold.ttf",
        "font/ttf",
        include_bytes!("../../horizon-theme/fonts/BricolageGrotesqueHeading-SemiBold.ttf"),
    ),
];

/// A file built into the binary, written after the head.
fn asset(content_type: &'static str, body: &'static [u8]) -> Response {
    let mut response = Response::text(200, content_type, "")
        .with("Content-Length", body.len().to_string())
        .with("Cache-Control", "public, max-age=86400");
    response.stream = Some(Box::new(move |mut stream: &std::net::TcpStream| {
        let _ = std::io::Write::write_all(&mut stream, body);
    }));
    response
}

/// The panel's address, its pending login codes and its sessions.
#[derive(Default)]
pub struct Auth {
    address: Option<SocketAddr>,
    public: Option<PublicAccess>,
    codes: Vec<(String, Instant)>,
    sessions: VecDeque<(String, Instant)>,
    failures: VecDeque<Instant>,
}

struct PublicAccess {
    origin: String,
    key: String,
}

impl PublicAccess {
    fn new(origin: String, key: String) -> Result<Self, String> {
        if !(32..=128).contains(&key.len()) || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(format!(
                "{PANEL_ACCESS_KEY_ENV} must contain a random access key of 32 to 128 printable ASCII characters without spaces"
            ));
        }
        Ok(Self { origin, key })
    }

    fn host(&self) -> &str {
        self.origin.trim_start_matches("https://")
    }
}

impl Auth {
    /// A one-time login link or the public address; `None` when the panel is off.
    pub fn login_link(&mut self) -> Option<String> {
        let address = self.address?;
        if let Some(public) = &self.public {
            return Some(public.origin.clone());
        }
        self.codes.retain(|(_, at)| at.elapsed() < LINK_LIFE);
        let code = api::random_token();
        self.codes.push((code.clone(), Instant::now()));
        Some(format!("http://{}/login?code={code}", reachable(address)))
    }

    /// Swaps a login code for a new session.
    fn redeem(&mut self, code: &str) -> Result<String, Response> {
        self.codes.retain(|(_, at)| at.elapsed() < LINK_LIFE);
        let Some(i) = self
            .codes
            .iter()
            .position(|(c, _)| api::same_token(c, code))
        else {
            let (status, message) = self.login_failure(
                "This login link was used already or has expired. Run `horizon-server panel` for a new one.",
            );
            return Err(page_error(status, message));
        };
        self.codes.remove(i);
        Ok(self.issue_session())
    }

    fn with_access_key(&mut self, key: &str) -> Result<String, Response> {
        if self
            .public
            .as_ref()
            .is_some_and(|public| api::same_token(&public.key, key))
        {
            return Ok(self.issue_session());
        }
        let (status, message) = self.login_failure("The access key is incorrect.");
        Err(Response::error(status, message))
    }

    fn login_failure(&mut self, invalid: &'static str) -> (u16, &'static str) {
        self.failures
            .retain(|at| at.elapsed() < Duration::from_secs(60));
        // Failed attempts must not lock the owner's valid, randomly generated key or code out.
        if self.failures.len() >= FAILURES_PER_MINUTE {
            return (429, "Too many failed logins. Wait a minute and try again.");
        }
        self.failures.push_back(Instant::now());
        (403, invalid)
    }

    fn issue_session(&mut self) -> String {
        if self.public.is_some() {
            self.sessions
                .retain(|(_, at)| at.elapsed() < PUBLIC_SESSION_LIFE);
        }
        let session = api::random_token();
        if self.sessions.len() == SESSIONS {
            self.sessions.pop_front();
        }
        self.sessions.push_back((session.clone(), Instant::now()));
        session
    }

    fn cookie_name(&self) -> &'static str {
        if self.public.is_some() {
            PUBLIC_COOKIE
        } else {
            COOKIE
        }
    }

    fn cookie(&self, session: Option<&str>) -> String {
        let mut cookie = format!(
            "{}={}; HttpOnly; SameSite=Strict; Path=/",
            self.cookie_name(),
            session.unwrap_or_default()
        );
        if self.public.is_some() {
            cookie.push_str("; Secure");
        }
        if session.is_none() {
            cookie.push_str("; Max-Age=0");
        } else if self.public.is_some() {
            cookie.push_str(&format!("; Max-Age={}", PUBLIC_SESSION_LIFE.as_secs()));
        }
        cookie
    }

    fn signed_in(&self, request: &Request) -> bool {
        request.cookie(self.cookie_name()).is_some_and(|s| {
            self.sessions.iter().any(|(token, at)| {
                (self.public.is_none() || at.elapsed() < PUBLIC_SESSION_LIFE)
                    && api::same_token(token, s)
            })
        })
    }

    fn sign_out(&mut self, request: &Request) {
        if let Some(s) = request.cookie(self.cookie_name()) {
            self.sessions.retain(|(t, _)| !api::same_token(t, s));
        }
    }
}

/// An address a browser can open for `address`: itself, or for every interface (`0.0.0.0`, `::`) this computer's
/// address on its network (the source of its default route), and loopback when it has none.
fn reachable(address: SocketAddr) -> SocketAddr {
    if !address.ip().is_unspecified() {
        return address;
    }
    let (any, probe, loopback): (IpAddr, SocketAddr, IpAddr) = if address.is_ipv4() {
        (
            Ipv4Addr::UNSPECIFIED.into(),
            (Ipv4Addr::new(192, 0, 2, 1), 9).into(),
            Ipv4Addr::LOCALHOST.into(),
        )
    } else {
        (
            Ipv6Addr::UNSPECIFIED.into(),
            (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 9).into(),
            Ipv6Addr::LOCALHOST.into(),
        )
    };
    // Connecting a UDP socket only picks the route; it sends nothing.
    let ip = UdpSocket::bind((any, 0))
        .and_then(|socket| socket.connect(probe).and_then(|()| socket.local_addr()))
        .map(|local| local.ip())
        .ok()
        .filter(|ip| !ip.is_unspecified())
        .unwrap_or(loopback);
    SocketAddr::new(ip, address.port())
}

fn page_error(status: u16, message: &str) -> Response {
    let message = match status {
        429 => "登录失败次数过多，请等一分钟再试。",
        403 => "登录链接已使用或已失效。请运行 horizon-server panel 获取新链接。",
        _ => message,
    };
    let body = format!(
        "<!doctype html><html lang=zh-CN><meta charset=utf-8><title>Horizon 服务端</title>\
         <link rel=stylesheet href=/panel.css><main class=message><p>{}</p></main>",
        message
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    );
    Response::text(status, "text/html; charset=utf-8", body)
}

/// The `Host` header names the panel (or the control API) itself: its address, `localhost` on loopback, or a tailnet name
/// (a MagicDNS short name or `*.ts.net`, with or without the port when `tailscale serve` passes it on). Bound to every
/// interface (`0.0.0.0` or `::`), any IP address with the bind's port names it too: a rebound DNS name is never an IP.
pub fn host_allowed(host: &str, address: SocketAddr) -> bool {
    if host == address.to_string() {
        return true;
    }
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(']') || name.starts_with('[') => (name, Some(port)),
        _ => (host, None),
    };
    let same_port = port == Some(&address.port().to_string());
    let any_interface = address.ip().is_unspecified();
    if (address.ip().is_loopback() || any_interface)
        && name.eq_ignore_ascii_case("localhost")
        && same_port
    {
        return true;
    }
    if any_interface && same_port && host.parse::<SocketAddr>().is_ok() {
        return true;
    }
    let tailnet_name = name.to_ascii_lowercase().ends_with(".ts.net")
        || (!name.is_empty()
            && !name.contains('.')
            && !name.contains('[')
            && !name.eq_ignore_ascii_case("localhost"));
    tailnet_name
        && (same_port || port.is_none())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn secure(response: Response) -> Response {
    let response = if response
        .headers
        .iter()
        .any(|(name, _)| *name == "Cache-Control")
    {
        response
    } else {
        response.with("Cache-Control", "no-store")
    };
    response
        .with(
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; \
             connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        )
        .with("X-Content-Type-Options", "nosniff")
        .with("X-Frame-Options", "DENY")
        .with("Referrer-Policy", "no-referrer")
}

fn handle(shared: &Arc<Shared>, address: SocketAddr, request: &Request) -> Response {
    let method = request.method.as_str();
    let path = request.path.as_str();
    let key_login = {
        let auth = shared.panel.lock().unwrap();
        let host = request.header("host").unwrap_or_default();
        if let Some(public) = &auth.public {
            if !host.eq_ignore_ascii_case(public.host()) {
                return Response::error(403, "wrong Host");
            }
            if method != "GET" && request.header("origin") != Some(public.origin.as_str()) {
                return Response::error(403, "wrong Origin");
            }
            true
        } else {
            if !host_allowed(host, address) {
                return Response::error(403, "wrong Host");
            }
            false
        }
    };
    match (method, path) {
        ("GET", "/") => return Response::text(200, "text/html; charset=utf-8", PAGE),
        ("GET", "/panel.js") => {
            return Response::text(200, "text/javascript; charset=utf-8", SCRIPT);
        }
        ("GET", "/zh-cn.js") => {
            return Response::text(200, "text/javascript; charset=utf-8", ZH_CN);
        }
        ("GET", "/panel.css") => return Response::text(200, "text/css; charset=utf-8", STYLE),
        ("GET", _) if let Some((_, kind, body)) = ASSETS.iter().find(|(p, _, _)| *p == path) => {
            return asset(kind, body);
        }
        ("GET", "/login/mode") => {
            return Response::json(200, &serde_json::json!({ "key": key_login }));
        }
        ("GET", "/login") if !key_login => {
            let code = request.param("code").unwrap_or_default();
            let mut auth = shared.panel.lock().unwrap();
            return match auth.redeem(code) {
                Ok(session) => Response::text(303, "text/plain", "")
                    .with("Location", "/")
                    .with("Set-Cookie", auth.cookie(Some(&session))),
                Err(response) => response,
            };
        }
        _ => {}
    }
    if method != "GET" && request.header("x-horizon") != Some("1") {
        return Response::error(403, "missing the X-Horizon header");
    }
    if key_login && (method, path) == ("POST", "/login") {
        #[derive(Deserialize)]
        struct Login {
            key: String,
        }
        let Ok(login) = serde_json::from_slice::<Login>(&request.body) else {
            return Response::error(400, "an access key is required");
        };
        let mut auth = shared.panel.lock().unwrap();
        return match auth.with_access_key(&login.key) {
            Ok(session) => Response::json(200, &serde_json::json!({ "ok": true }))
                .with("Set-Cookie", auth.cookie(Some(&session))),
            Err(response) => response,
        };
    }
    if (method, path) == ("POST", "/logout") {
        let mut auth = shared.panel.lock().unwrap();
        auth.sign_out(request);
        return Response::json(200, &serde_json::json!({ "ok": true }))
            .with("Set-Cookie", auth.cookie(None));
    }
    if !path.starts_with("/api/v1/") {
        return Response::error(404, "no such page");
    }
    if !shared.panel.lock().unwrap().signed_in(request) {
        let message = if key_login {
            "sign in with the panel access key"
        } else {
            "sign in with a link from `horizon-server panel` on the server"
        };
        return Response::error(401, message);
    }
    if path == "/api/v1/panel/login" {
        return Response::error(403, "login links come from `horizon-server panel` only");
    }
    match (method, path) {
        ("GET", "/api/v1/panel/plugins") => plugins::list(shared),
        ("POST", "/api/v1/panel/plugins/action") => plugins::run(shared, &request.body),
        ("GET" | "PATCH", "/api/v1/panel/plugins/config") => plugins::settings(
            shared,
            request.param("plugin").unwrap_or_default(),
            (method == "PATCH").then_some(request.body.as_slice()),
        ),
        _ => api::route(shared, request),
    }
}

/// Starts the panel on `address`. A public origin requires the access key before it can listen.
pub fn start(
    shared: Arc<Shared>,
    address: SocketAddr,
    origin: Option<String>,
) -> Result<(), String> {
    let public = origin
        .map(|origin| {
            let key = std::env::var(PANEL_ACCESS_KEY_ENV)
                .map_err(|_| format!("{PANEL_ACCESS_KEY_ENV} is required for panel.public_url"))?;
            PublicAccess::new(origin, key)
        })
        .transpose()?;
    start_with_access(shared, address, public)
}

fn start_with_access(
    shared: Arc<Shared>,
    address: SocketAddr,
    public: Option<PublicAccess>,
) -> Result<(), String> {
    let listener = TcpListener::bind(address).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    *shared.panel.lock().unwrap() = Auth {
        address: Some(address),
        public,
        ..Auth::default()
    };
    http::serve(listener, move |request| {
        secure(handle(&shared, address, request))
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// docs/server.md section 6: a login link's code is "good for 10 minutes", and wrong codes are counted
    /// over a minute ("after 10 failed logins in a minute, logins are refused for a while").
    #[test]
    fn a_login_code_lasts_ten_minutes_and_failures_count_for_one() {
        let ago = |seconds| {
            Instant::now()
                .checked_sub(Duration::from_secs(seconds))
                .unwrap()
        };
        let status = |r: Result<String, Response>| r.map_err(|e| e.status);
        let mut auth = Auth {
            address: Some("127.0.0.1:8130".parse().unwrap()),
            ..Auth::default()
        };
        // A code a second short of ten minutes old still signs in; one a second past does not.
        auth.codes.push(("fresh".into(), ago(599)));
        auth.codes.push(("stale".into(), ago(601)));
        assert!(auth.redeem("fresh").is_ok());
        assert_eq!(status(auth.redeem("stale")), Err(403));
        // Ten failures in the last minute refuse the eleventh try with 429, even a wrong one...
        auth.failures = (0..FAILURES_PER_MINUTE).map(|_| ago(59)).collect();
        assert_eq!(status(auth.redeem("nope")), Err(429));
        // ...and failures older than a minute no longer count.
        auth.failures = (0..FAILURES_PER_MINUTE).map(|_| ago(61)).collect();
        assert_eq!(status(auth.redeem("nope")), Err(403));
    }

    #[test]
    fn hosts() {
        let local: SocketAddr = "127.0.0.1:8130".parse().unwrap();
        assert!(host_allowed("127.0.0.1:8130", local));
        assert!(host_allowed("localhost:8130", local));
        assert!(!host_allowed("localhost:8131", local));
        assert!(!host_allowed("evil.example:8130", local));
        assert!(!host_allowed("127.0.0.1.nip.io:8130", local));
        let tailnet: SocketAddr = "100.100.1.2:8130".parse().unwrap();
        assert!(host_allowed("100.100.1.2:8130", tailnet));
        assert!(host_allowed("myserver:8130", tailnet));
        assert!(host_allowed("myserver.tail1234.ts.net", tailnet));
        assert!(!host_allowed("localhost:8130", tailnet));
        assert!(!host_allowed("evil.example", tailnet));
        let v6: SocketAddr = "[::1]:8130".parse().unwrap();
        assert!(host_allowed("[::1]:8130", v6));
        assert!(!host_allowed("[::2]:8130", v6));
        // Bound to every interface (#12039): any IP address on the bind's port, never a DNS name.
        let lan: SocketAddr = "0.0.0.0:8130".parse().unwrap();
        assert!(host_allowed("192.168.1.20:8130", lan));
        assert!(host_allowed("127.0.0.1:8130", lan));
        assert!(host_allowed("localhost:8130", lan));
        assert!(!host_allowed("192.168.1.20:8131", lan));
        assert!(!host_allowed("192.168.1.20", lan));
        assert!(!host_allowed("evil.example:8130", lan));
        assert!(!host_allowed("192.168.1.20.nip.io:8130", lan));
        let lan6: SocketAddr = "[::]:8130".parse().unwrap();
        assert!(host_allowed("[fd00::20]:8130", lan6));
        assert!(host_allowed("192.168.1.20:8130", lan6));
        assert!(!host_allowed("[fd00::20]:8131", lan6));
        assert!(!host_allowed("evil.example:8130", lan6));
    }

    #[test]
    #[ignore = "daily"]
    fn login_links_name_an_address_a_browser_can_open() {
        let tailnet: SocketAddr = "100.100.1.2:8130".parse().unwrap();
        assert_eq!(reachable(tailnet), tailnet);
        for bind in ["0.0.0.0:8130", "[::]:8130"] {
            let bind: SocketAddr = bind.parse().unwrap();
            let link = reachable(bind);
            assert!(!link.ip().is_unspecified(), "{link}");
            assert_eq!(link.port(), 8130);
            assert_eq!(link.is_ipv4(), bind.is_ipv4());
            // The link it prints passes the panel's own Host check.
            assert!(host_allowed(&link.to_string(), bind), "{link}");
        }
        let mut auth = Auth {
            address: Some("0.0.0.0:8130".parse().unwrap()),
            ..Auth::default()
        };
        let url = auth.login_link().unwrap();
        assert!(!url.starts_with("http://0.0.0.0:"), "{url}");
    }

    /// One raw request to the panel; the status line, the headers and the body.
    fn send(address: SocketAddr, text: &str) -> (String, String) {
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream.write_all(text.as_bytes()).unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).unwrap();
        let (head, body) = out.split_once("\r\n\r\n").unwrap();
        (head.to_owned(), body.to_owned())
    }

    #[test]
    fn public_panel_key_issues_an_https_session_with_origin_checks() {
        const ORIGIN: &str = "https://panel.example.test";
        let key = api::random_token();
        assert!(PublicAccess::new(ORIGIN.into(), "short".into()).is_err());
        let public = PublicAccess::new(ORIGIN.into(), key.clone()).unwrap();
        let shared = Shared::with_install(std::path::Path::new("."), None);
        start_with_access(shared.clone(), "127.0.0.1:0".parse().unwrap(), Some(public)).unwrap();
        let address = shared.panel.lock().unwrap().address.unwrap();
        let call = |method: &str, path: &str, host: &str, headers: &str, body: &str| {
            send(
                address,
                &format!(
                    "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                ),
            )
        };
        let host = "panel.example.test";
        let get = |path: &str, headers: &str| call("GET", path, host, headers, "");
        let post = |path: &str, headers: &str, body: &str| call("POST", path, host, headers, body);
        let scope = format!("Origin: {ORIGIN}\r\nX-Horizon: 1\r\n");
        let credentials = serde_json::json!({ "key": key }).to_string();

        assert_eq!(
            shared.panel.lock().unwrap().login_link().as_deref(),
            Some(ORIGIN)
        );
        assert!(get("/login?code=ignored", "").0.starts_with("HTTP/1.1 404"));
        assert!(
            get("/api/v1/panel/plugins", "")
                .0
                .starts_with("HTTP/1.1 401")
        );
        assert!(
            call("GET", "/", "other.example.test", "", "")
                .0
                .starts_with("HTTP/1.1 403")
        );
        assert!(
            post(
                "/login",
                "X-Horizon: 1\r\nOrigin: http://panel.example.test\r\n",
                &credentials
            )
            .0
            .starts_with("HTTP/1.1 403")
        );
        assert!(
            post("/login", &format!("Origin: {ORIGIN}\r\n"), &credentials)
                .0
                .starts_with("HTTP/1.1 403")
        );
        assert!(
            post("/login", &scope, r#"{"key":"incorrect"}"#)
                .0
                .starts_with("HTTP/1.1 403")
        );

        let (head, body) = post("/login", &scope, &credentials);
        assert!(head.starts_with("HTTP/1.1 200"), "{head} {body}");
        assert!(head.contains("Cache-Control: no-store"));
        let cookie = head
            .lines()
            .find_map(|line| line.strip_prefix("Set-Cookie: "))
            .unwrap();
        assert!(cookie.starts_with("__Host-horizon_panel="));
        assert!(cookie.contains("; HttpOnly; SameSite=Strict; Path=/; Secure; Max-Age=43200"));
        let cookie_header = format!("Cookie: {}\r\n", cookie.split(';').next().unwrap());
        assert!(
            get("/api/v1/status", &cookie_header)
                .0
                .starts_with("HTTP/1.1 200")
        );
        assert!(
            post(
                "/api/v1/panel/plugins/action",
                &format!("X-Horizon: 1\r\n{cookie_header}"),
                "{}"
            )
            .0
            .starts_with("HTTP/1.1 403")
        );
        assert!(
            post("/logout", &format!("{scope}{cookie_header}"), "")
                .0
                .contains("; Max-Age=0")
        );
        assert!(
            get("/api/v1/status", &cookie_header)
                .0
                .starts_with("HTTP/1.1 401")
        );
    }

    #[test]
    #[ignore = "daily"]
    fn sign_in_with_a_link() {
        let dir = std::env::temp_dir().join(format!("horizon-server-panel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Shared::new(&dir);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        start(shared.clone(), address, None).unwrap();
        let host = format!("Host: {address}\r\n");
        let get = |path: &str, extra: &str| {
            send(
                address,
                &format!("GET {path} HTTP/1.1\r\n{host}{extra}Connection: close\r\n\r\n"),
            )
        };
        // The page itself is public; the API is not.
        let (head, body) = get("/", "");
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Security-Policy"));
        assert!(body.contains("panel.js"));
        assert!(get("/api/v1/status", "").0.starts_with("HTTP/1.1 401"));
        let rebound = send(
            address,
            "GET / HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
        );
        assert!(rebound.0.starts_with("HTTP/1.1 403"));
        // A link works once.
        let url = shared.panel.lock().unwrap().login_link().unwrap();
        let path = url.split_once(&address.to_string()).unwrap().1.to_owned();
        let (head, _) = get(&path, "");
        assert!(head.starts_with("HTTP/1.1 303"), "{head}");
        let cookie = head
            .lines()
            .find_map(|l| l.strip_prefix("Set-Cookie: "))
            .unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        let session = cookie.split(';').next().unwrap().to_owned();
        assert!(get(&path, "").0.starts_with("HTTP/1.1 403"));
        let with_cookie = format!("Cookie: other=1; {session}\r\n");
        let (head, body) = get("/api/v1/status", &with_cookie);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(body.contains("\"phase\":\"starting\""));
        // Changes need the header a foreign page can't send.
        let post = |extra: &str| {
            send(
                address,
                &format!(
                    "POST /api/v1/restart HTTP/1.1\r\n{host}{with_cookie}{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
            )
            .0
        };
        assert!(post("").starts_with("HTTP/1.1 403"));
        assert!(!shared.restart.load(std::sync::atomic::Ordering::SeqCst));
        assert!(post("X-Horizon: 1\r\n").starts_with("HTTP/1.1 200"));
        assert!(shared.restart.load(std::sync::atomic::Ordering::SeqCst));
        // Login links come only from the token API.
        assert!(
            send(
                address,
                &format!(
                    "POST /api/v1/panel/login HTTP/1.1\r\n{host}{with_cookie}X-Horizon: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
            )
            .0
            .starts_with("HTTP/1.1 403")
        );
        // Wrong codes are counted (the used link above was one), then refused for a while.
        for _ in 1..FAILURES_PER_MINUTE {
            assert!(get("/login?code=nope", "").0.starts_with("HTTP/1.1 403"));
        }
        assert!(get("/login?code=nope", "").0.starts_with("HTTP/1.1 429"));
        // A real link still signs in while the wrong ones are refused.
        let url = shared.panel.lock().unwrap().login_link().unwrap();
        let path = url.split_once(&address.to_string()).unwrap().1.to_owned();
        assert!(get(&path, "").0.starts_with("HTTP/1.1 303"));
        assert!(get("/login?code=nope", "").0.starts_with("HTTP/1.1 429"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
