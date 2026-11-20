//! PigeonAuth's local service and the admission screen before any player state exists.
use horizon_core::nbt::Tag;
use horizon_protocol::{
    connection::Connection,
    packets::{Empty, configuration as cfg, play},
};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

pub const MOD_ID: &str = "pigeonauth";
pub const VERSION: &str = "0.1.0";
const URL_ENV: &str = "HORIZON_PIGEONAUTH_URL";
const HOST_ENV: &str = "HORIZON_PIGEONAUTH_OFFLINE_HOST";
const KEY_ENV: &str = "HORIZON_PIGEONAUTH_KEY";
const SUBMIT: &str = "pigeonauth:submit";
pub(super) const SET: &str = "pigeonauth:set";
pub(super) const UNAVAILABLE: Message =
    Message("unavailable", "PigeonAuth 暂时无法处理登录，请稍后再试。");
pub(super) const RESERVED: Message = Message(
    "reserved",
    "此 ID 归正版玩家所有。请从正版入口登录后设置离线密码。",
);
const RETRY: Message = Message("retry", "账号状态已变更，请重新连接。");
const BUSY: Message = Message("busy", "尝试次数过多，请稍后再试。");
const TIMEOUT: Message = Message("timeout", "认证超时，请重新连接。");
const FORMAT: Message = Message("password_format", "请输入 10–64 个字符的服务器专用密码。");
const WRONG: Message = Message("wrong_password", "密码错误，请再试一次。");
const REGISTERED: Message = Message("registered", "注册成功。请重新输入密码完成登录。");
const SET_OK: Message = Message(
    "set_ok",
    "[PigeonAuth] 已设置离线密码。改名后请用新的正版 ID 连接。",
);

#[derive(Clone, Copy)]
pub(super) struct Message(&'static str, &'static str);
impl Message {
    pub fn tag(self) -> Tag {
        Tag::compound([
            ("translate", string(&format!("pigeonauth.{}", self.0))),
            ("fallback", string(self.1)),
        ])
    }
}

fn string(value: &str) -> Tag {
    Tag::String(value.into())
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum Decision {
    Ready,
    Prompt,
    Register { uuid: String },
    Login { uuid: String },
    Denied,
    BadPassword,
    BadFormat,
    Retry,
    Busy,
}

pub struct Config {
    host: String,
    endpoint: String,
    key: String,
    agent: ureq::Agent,
}

impl Config {
    /// Dedicated-only. Installations without this environment retain vanilla authentication.
    pub fn from_env(online: bool) -> Result<Option<Self>, String> {
        if [URL_ENV, HOST_ENV, KEY_ENV]
            .iter()
            .all(|env| std::env::var_os(env).is_none())
        {
            return Ok(None);
        }
        let var = |name| std::env::var(name).map_err(|_| format!("PigeonAuth requires {name}"));
        let url = var(URL_ENV)?;
        let address: SocketAddr = url
            .strip_prefix("http://")
            .and_then(|addr| addr.parse().ok())
            .filter(|address: &SocketAddr| address.ip().is_loopback() && address.port() != 0)
            .ok_or_else(|| {
                format!("{URL_ENV} requires a loopback HTTP origin with an explicit port")
            })?;
        let host = var(HOST_ENV)?.trim_end_matches('.').to_ascii_lowercase();
        if !host.contains('.')
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            || host.len() > 253
            || host.parse::<IpAddr>().is_ok()
        {
            return Err(format!("{HOST_ENV} requires a distinct DNS hostname"));
        }
        let key = var(KEY_ENV)?;
        if key.len() < 32 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(format!(
                "{KEY_ENV} requires at least 32 printable ASCII characters without spaces"
            ));
        }
        if !online {
            return Err(
                "PigeonAuth requires online-mode=true for the verified Minecraft login route"
                    .into(),
            );
        }
        Ok(Some(Self {
            host,
            endpoint: format!("http://{address}/v1/auth"),
            key,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(7))
                .redirects(0)
                .build(),
        }))
    }

    pub(super) fn offline_host(&self, host: &str) -> bool {
        host.trim_end_matches('.').eq_ignore_ascii_case(&self.host)
    }

    pub(super) fn request(
        &self,
        action: &str,
        name: &str,
        uuid: u128,
        ip: IpAddr,
        password: Option<&str>,
    ) -> Result<Decision, Message> {
        #[derive(serde::Serialize)]
        struct Request<'a> {
            action: &'a str,
            name: &'a str,
            uuid: String,
            ip: IpAddr,
            password: Option<&'a str>,
        }
        let body = serde_json::to_vec(&Request {
            action,
            name,
            uuid: format!("{uuid:032x}"),
            ip,
            password,
        })
        .map_err(|_| UNAVAILABLE)?;
        let mut request = self.agent.post(&self.endpoint);
        if action == "observe" {
            request = request.timeout(Duration::from_secs(1));
        }
        let response = request
            .set("Authorization", &format!("Bearer {}", self.key))
            .set("Content-Type", "application/json")
            .send_bytes(&body)
            .map_err(|_| UNAVAILABLE)?;
        serde_json::from_reader(std::io::Read::take(response.into_reader(), 1024))
            .map_err(|_| UNAVAILABLE)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Form {
    Register,
    Login,
    Set,
}

fn form(kind: Form, error: Option<Message>) -> Tag {
    let (title, button, action) = match kind {
        Form::Register => (
            Message("register", "注册离线账号"),
            Message("submit_register", "注册"),
            SUBMIT,
        ),
        Form::Login => (
            Message("login", "登录离线账号"),
            Message("submit_login", "登录"),
            SUBMIT,
        ),
        Form::Set => (
            Message("set", "设置离线登录密码"),
            Message("submit_set", "保存密码"),
            SET,
        ),
    };
    Tag::compound([
        ("type", string("minecraft:multi_action")),
        ("title", title.tag()),
        (
            "can_close_with_escape",
            Tag::Byte(i8::from(matches!(kind, Form::Set))),
        ),
        // Close before later configuration prompts such as a resource pack or rules.
        ("after_action", string("close")),
        (
            "body",
            Tag::compound([
                ("type", string("minecraft:plain_message")),
                ("contents", error.unwrap_or(FORMAT).tag()),
            ]),
        ),
        (
            "inputs",
            Tag::List(vec![Tag::compound([
                ("type", string("minecraft:text")),
                ("key", string("password")),
                ("label", Message("password", "密码").tag()),
                ("max_length", Tag::Int(64)),
            ])]),
        ),
        (
            "actions",
            Tag::List(vec![Tag::compound([
                ("label", button.tag()),
                (
                    "action",
                    Tag::compound([
                        ("type", string("minecraft:dynamic/custom")),
                        ("id", string(action)),
                    ]),
                ),
            ])]),
        ),
    ])
}

pub(super) fn show_setup(sender: &super::PlayerSender, error: Option<Message>) {
    sender.send(setup_dialog(error).encode());
}

fn setup_dialog(error: Option<Message>) -> play::Clientbound {
    play::Clientbound::ShowDialog(play::ShowDialog {
        dialog: play::DialogHolder::Direct(form(Form::Set, error)),
    })
}

pub(super) fn send_set_result(sender: &super::PlayerSender, result: Result<Decision, Message>) {
    let message = match result {
        Ok(Decision::Ready) => SET_OK,
        Ok(Decision::BadFormat) => return show_setup(sender, Some(FORMAT)),
        Ok(Decision::Denied) => RETRY,
        _ => UNAVAILABLE,
    };
    send_chat(sender, message.tag());
}

fn send_chat(sender: &super::PlayerSender, content: Tag) {
    sender.send(
        play::Clientbound::SystemChat(play::SystemChat {
            content,
            overlay: false,
        })
        .encode(),
    );
}

pub(super) fn password(click: &cfg::CustomClickAction) -> Option<String> {
    let Tag::String(value) = click.payload.as_ref()?.get("password")? else {
        return None;
    };
    let password = value.to_string_lossy();
    (password.len() <= 256).then_some(password)
}

type BufferedPackets = VecDeque<(i32, Vec<u8>)>;

pub(super) struct Pending<'a> {
    pub config: &'a Config,
    pub address: SocketAddr,
    pub form: Form,
    pub admitted: bool,
}

impl Pending<'_> {
    /// Hold the client in configuration. Only PigeonAuth can advance the connection;
    /// ordinary client metadata waits here and mod callbacks cannot run yet.
    pub fn authenticate(
        &mut self,
        c: &mut Connection,
        name: &str,
        uuid: u128,
    ) -> Result<BufferedPackets, Message> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut packets = VecDeque::new();
        let mut error = None;
        for _ in 0..5 {
            c.send(
                &cfg::Clientbound::ShowDialog(cfg::ShowDialog {
                    dialog: form(self.form, error),
                })
                .encode(),
            )
            .map_err(|_| TIMEOUT)?;
            let password = loop {
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(25));
                if remaining.is_zero() {
                    return Err(TIMEOUT);
                }
                c.reader
                    .set_read_timeout(Some(remaining))
                    .map_err(|_| TIMEOUT)?;
                let (id, bytes) = c.recv().map_err(|_| TIMEOUT)?;
                if bytes.len() > 8192 {
                    return Err(RETRY);
                }
                let packet = cfg::Serverbound::decode(id, &bytes).map_err(|_| RETRY)?;
                match packet {
                    cfg::Serverbound::CustomClickAction(click) if click.id == SUBMIT => {
                        break password(&click);
                    }
                    cfg::Serverbound::ClientInformation(_) | cfg::Serverbound::CustomPayload(_)
                        if packets.len() < 16 =>
                    {
                        packets.push_back((id, bytes))
                    }
                    _ => return Err(RETRY),
                }
            };
            let Some(password) = password else {
                error = Some(FORMAT);
                continue;
            };
            let action = if matches!(self.form, Form::Register) {
                "register"
            } else {
                "verify"
            };
            match self
                .config
                .request(action, name, uuid, self.address.ip(), Some(&password))?
            {
                Decision::Ready if matches!(self.form, Form::Login) => {
                    c.send(&cfg::Clientbound::ClearDialog(Empty).encode())
                        .map_err(|_| TIMEOUT)?;
                    return Ok(packets);
                }
                Decision::Login { uuid: account }
                    if matches!(self.form, Form::Register)
                        && u128::from_str_radix(&account, 16) == Ok(uuid) =>
                {
                    self.form = Form::Login;
                    error = Some(REGISTERED);
                }
                Decision::BadPassword => error = Some(WRONG),
                Decision::BadFormat => error = Some(FORMAT),
                Decision::Denied => return Err(RESERVED),
                Decision::Busy => return Err(BUSY),
                _ => return Err(RETRY),
            }
        }
        Err(BUSY)
    }
}

pub(super) fn begin(
    config: &Config,
    name: &str,
    address: SocketAddr,
) -> Result<(u128, Form), Message> {
    // Namespace separate from pre-PigeonAuth playerdata, which did not prove ownership.
    let candidate = horizon_protocol::crypto::offline_uuid(&format!(
        "PigeonAuth:offline:{}",
        name.to_ascii_lowercase()
    ));
    let (uuid, form) = match config.request("begin", name, candidate, address.ip(), None)? {
        Decision::Register { uuid } => (uuid, Form::Register),
        Decision::Login { uuid } => (uuid, Form::Login),
        Decision::Denied => return Err(RESERVED),
        Decision::Busy => return Err(BUSY),
        _ => return Err(UNAVAILABLE),
    };
    u128::from_str_radix(&uuid, 16)
        .map(|uuid| (uuid, form))
        .map_err(|_| UNAVAILABLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::{Lan, NetConfig, OnlineMode, data::SyncData};
    use horizon_protocol::{
        PROTOCOL_VERSION,
        crypto::{generate_secret, offline_uuid},
        packets::{handshake, login},
        rsa::RsaPublicKey,
    };
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        sync::Arc,
    };

    #[test]
    fn verified_players_receive_a_dismissible_password_dialog_in_play() {
        let play::Clientbound::ShowDialog(play::ShowDialog {
            dialog: play::DialogHolder::Direct(dialog),
        }) = setup_dialog(None)
        else {
            panic!("expected the setup dialog to open directly");
        };
        assert_eq!(
            dialog.get("title").unwrap().get("translate"),
            Some(&string("pigeonauth.set"))
        );
        let dialog = crate::dialog::Dialog::parse(&dialog).expect("valid play dialog");
        assert!(dialog.can_close_with_escape);
        assert_eq!(dialog.inputs.len(), 1);
    }

    #[test]
    fn offline_player_stays_out_of_the_world_until_register_then_login() {
        let backend = TcpListener::bind("127.0.0.1:0").unwrap();
        let auth = Config {
            host: "offline.example.com".into(),
            endpoint: format!("http://{}/v1/auth", backend.local_addr().unwrap()),
            key: "test-key".into(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(3))
                .build(),
        };
        std::thread::scope(|scope| {
            scope.spawn(move || {
                backend.set_nonblocking(true).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                for i in 0..4 {
                    let mut stream = loop {
                        match backend.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < deadline, "no PigeonAuth request received");
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(error) => panic!("accept: {error}"),
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut reader = BufReader::new(&mut stream);
                    let mut length = None;
                    loop {
                        let mut line = String::new();
                        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                        if line == "\r\n" { break; }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = Some(value.trim().parse().unwrap());
                        }
                    }
                    let mut body = vec![0; length.unwrap()];
                    reader.read_exact(&mut body).unwrap();
                    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    let result = match i {
                        0 => {
                            assert_eq!(request["action"], "begin");
                            serde_json::json!({"status":"register", "uuid":request["uuid"]})
                        }
                        1 => {
                            assert_eq!(request["action"], "register");
                            serde_json::json!({"status":"login", "uuid":request["uuid"]})
                        }
                        _ => {
                            assert_eq!(request["action"], "verify");
                            let status = if request["password"] == "my-test-password" { "ready" } else { "bad_password" };
                            serde_json::json!({"status":status})
                        }
                    }.to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}", result.len()).unwrap();
                }
            });
            let config = NetConfig {
                online: Some(Arc::new(OnlineMode::generate(false).unwrap())),
                pigeon_auth: Some(Arc::new(auth)),
                listen: false,
                ..NetConfig::loopback(0, "PigeonAuth".into(), 8)
            };
            let mut lan = Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let listener = Lan::bind(Some(std::net::Ipv4Addr::LOCALHOST.into()), 0).unwrap();
            let address = listener.local_addr().unwrap();
            lan.listen_on(listener, false).unwrap();
            let mut client = Connection::from_stream(TcpStream::connect(address).unwrap()).unwrap();
            client
                .reader
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            client
                .send(
                    &handshake::Serverbound::Intention(handshake::Intention {
                        protocol: PROTOCOL_VERSION,
                        host: "Offline.Example.Com".into(),
                        port: address.port(),
                        intent: handshake::LOGIN,
                    })
                    .encode(),
                )
                .unwrap();
            client
                .send(
                    &login::Serverbound::Hello(login::ServerHello {
                        name: "MockPlayer".into(),
                        uuid: offline_uuid("MockPlayer"),
                    })
                    .encode(),
                )
                .unwrap();
            let (id, raw) = client.recv().unwrap();
            let login::Clientbound::Hello(welcome) = login::Clientbound::decode(id, &raw).unwrap()
            else {
                panic!("expected encrypted login challenge")
            };
            assert!(!welcome.should_authenticate);
            let key = RsaPublicKey::from_der(&welcome.public_key).unwrap();
            let secret = generate_secret().unwrap();
            client
                .send(
                    &login::Serverbound::Key(login::Key {
                        key: key.encrypt(&secret).unwrap(),
                        challenge: key.encrypt(&welcome.challenge).unwrap(),
                    })
                    .encode(),
                )
                .unwrap();
            client.enable_encryption(&secret);
            loop {
                let (id, raw) = client.recv().unwrap();
                match login::Clientbound::decode(id, &raw).unwrap() {
                    login::Clientbound::Compression(p) => client.set_compression(p.threshold),
                    login::Clientbound::LoginFinished(p) => {
                        assert_ne!(p.profile.uuid, offline_uuid("MockPlayer"));
                        break;
                    }
                    packet => panic!("unexpected packet {packet:?}"),
                }
            }
            client
                .send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                .unwrap();
            let read = |client: &mut Connection| {
                let (id, raw) = client.recv().unwrap();
                cfg::Clientbound::decode(id, &raw).unwrap()
            };
            for (password, expected_title) in [
                ("my-test-password", "pigeonauth.register"),
                ("bad-test-password", "pigeonauth.login"),
                ("my-test-password", "pigeonauth.login"),
            ] {
                let cfg::Clientbound::ShowDialog(p) = read(&mut client) else {
                    panic!("expected a dialog before any world data")
                };
                let dialog = crate::dialog::Dialog::parse(&p.dialog).expect("valid client dialog");
                assert_eq!(dialog.inputs.len(), 1);
                assert!(!dialog.can_close_with_escape);
                assert_eq!(dialog.after_action, crate::dialog::AfterAction::Close);
                assert_eq!(
                    p.dialog.get("title").unwrap().get("translate"),
                    Some(&string(expected_title))
                );
                assert_eq!(lan.shared.online(), 0);
                assert!(lan.poll().is_empty());
                client
                    .send(
                        &cfg::Serverbound::CustomClickAction(cfg::CustomClickAction {
                            id: SUBMIT.into(),
                            payload: Some(Tag::compound([("password", string(password))])),
                        })
                        .encode(),
                    )
                    .unwrap();
            }
            assert!(matches!(
                read(&mut client),
                cfg::Clientbound::ClearDialog(_)
            ));
            loop {
                match read(&mut client) {
                    cfg::Clientbound::SelectKnownPacks(p) => client
                        .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                        .unwrap(),
                    cfg::Clientbound::FinishConfiguration(_) => {
                        client
                            .send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                            .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
            assert_eq!(lan.shared.online(), 1);
        });
    }
}
