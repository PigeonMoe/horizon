// The Horizon server panel (docs/server.md, section 6). It polls the control API once a second through the
// panel's session; every change carries the X-Horizon header. Text from the server is only ever set with
// textContent, never as HTML.
"use strict";

const $ = (id) => document.getElementById(id);

function localizeDocument() {
  for (const el of document.querySelectorAll("[data-t]")) el.textContent = t(el.dataset.t);
  for (const attribute of ["placeholder", "aria-label"]) {
    for (const el of document.querySelectorAll(`[data-t-${attribute}]`)) {
      el.setAttribute(attribute, t(el.getAttribute(`data-t-${attribute}`)));
    }
  }
}

// --- Talking to the server -------------------------------------------------------------------------------

class SignedOut extends Error {}

async function api(method, path, body) {
  let response;
  try {
    response = await fetch(path, {
      method,
      credentials: "same-origin",
      headers: { "X-Horizon": "1", "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (e) {
    throw new Error(t("the server isn't answering"));
  }
  if (response.status === 401) throw new SignedOut();
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(value.error ? t(value.error) : t("error {status}", { status: response.status }));
  return value;
}

/// Runs a change and says how it went; returns the reply, or null when it failed.
async function act(method, path, body, done) {
  try {
    const reply = await api(method, path, body);
    const said = reply && Array.isArray(reply.output) ? reply.output.join("\n") : "";
    toast(said || done || t("Done."));
    return reply;
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    else toast(e.message, true);
    return null;
  }
}

// --- Small helpers -------------------------------------------------------------------------------------

/// An element: h("li", { class: "x", onclick }, child, "text", ...).
function h(tag, props, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props || {})) {
    if (value == null || value === false) continue;
    if (key.startsWith("on")) el.addEventListener(key.slice(2), value);
    else if (key === "class") el.className = value;
    else el.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

function icon(name) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
}

function button(label, action, { kind, iconName, small = true, title } = {}) {
  return h(
    "button",
    { type: "button", class: [kind, small && "small"].filter(Boolean).join(" "), title, "aria-label": title, onclick: action },
    iconName && icon(iconName),
    label && h("span", { class: "label" }, label),
  );
}

function duration(seconds) {
  if (seconds == null) return "";
  const d = Math.floor(seconds / 86400), hr = Math.floor(seconds / 3600) % 24, m = Math.floor(seconds / 60) % 60;
  if (d) return t("{d} d {hr} h", { d, hr });
  if (hr) return t("{hr} h {m} min", { hr, m });
  return m ? t("{m} min", { m }) : t("{seconds} s", { seconds });
}

function bytes(n) {
  if (n == null) return "–";
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.ceil(n / 1e3))} kB`;
}

/// "2026-11-04_18-00-00-name.zip" (backup::file_name) → a date and the name given.
function backupParts(name) {
  const m = /^(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})-?(.*?)(\.zip)?$/.exec(name);
  if (!m) return { when: null, label: name.replace(/\.zip$/, "") };
  const when = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  return { when, label: m[7] };
}

const dateFormat = new Intl.DateTimeFormat(document.documentElement.lang, { dateStyle: "medium", timeStyle: "short" });
function ago(date) {
  const s = Math.round((Date.now() - date.getTime()) / 1000);
  if (s < 90) return t("just now");
  if (s < 3600) return t("{minutes} min ago", { minutes: Math.round(s / 60) });
  if (s < 86400) return t("{hours} h ago", { hours: Math.round(s / 3600) });
  return t("{days} d ago", { days: Math.round(s / 86400) });
}

/// A steady colour per player name, for the letter avatar (the panel loads no skins from elsewhere).
function avatar(name) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.codePointAt(0)) >>> 0;
  const el = h("span", { class: "avatar", "aria-hidden": "true" }, name.slice(0, 1));
  el.style.background = `hsl(${hash % 360} 55% 42%)`;
  return el;
}

function ping(ms) {
  const quality = ms == null ? "" : ms < 80 ? "good" : ms < 200 ? "fair" : "poor";
  return h("span", { class: "row-side" }, h("span", { class: `ping ${quality}`, "aria-hidden": "true" }, h("i"), h("i"), h("i")), ms == null ? "–" : `${ms} ms`);
}

function fill(listId, emptyId, items) {
  $(listId).replaceChildren(...items);
  if (emptyId) $(emptyId).hidden = items.length > 0;
}

// --- Toasts and the dialog ------------------------------------------------------------------------------

function toast(text, error) {
  const el = h("div", { class: `toast${error ? " error" : ""}` }, text);
  $("toasts").append(el);
  setTimeout(() => el.remove(), error ? 7000 : 4000);
  while ($("toasts").children.length > 3) $("toasts").firstChild.remove();
}

/// Asks before a change: resolves to the fields' values (an object), or null when cancelled.
function ask({ title, text, ok = t("OK"), danger = false, fields = [] }) {
  const dialog = $("dialog");
  $("dialog-title").textContent = title;
  $("dialog-text").textContent = text || "";
  $("dialog-text").hidden = !text;
  $("dialog-ok").textContent = ok;
  $("dialog-ok").className = danger ? "primary destructive-fill" : "primary";
  const inputs = fields.map((f) => {
    const input = f.options
      ? h("select", { name: f.name }, f.options.map(([value, label]) => h("option", { value }, label)))
      : h("input", { name: f.name, placeholder: f.placeholder || "", maxlength: f.max || 200, autocomplete: "off" });
    return [f.name, input, h("label", {}, f.label, input)];
  });
  $("dialog-fields").replaceChildren(...inputs.map((i) => i[2]));
  dialog.returnValue = "";
  dialog.showModal();
  (inputs[0] ? inputs[0][1] : $("dialog-ok")).focus();
  return new Promise((resolve) => {
    dialog.addEventListener(
      "close",
      () => resolve(dialog.returnValue === "ok" ? Object.fromEntries(inputs.map(([n, i]) => [n, i.value.trim()])) : null),
      { once: true },
    );
  });
}

// --- Pages ----------------------------------------------------------------------------------------------

const PAGES = {
  overview: [t("Overview"), t("How the server is doing right now.")],
  players: [t("Players"), t("Who is online, who may join and who is banned.")],
  console: [t("Console"), t("The server's log as it happens. Commands run as if typed in the server's console.")],
  backups: [t("Backups"), t("Zips of the world and the settings in backups/. Restoring keeps the current world as a backup.")],
  settings: [t("Server settings"), t("settings.lead")],
  plugins: [t("Plugins"), ""],
  reports: [t("Crash reports"), t("The game writes one each time it stops unexpectedly.")],
};
let page = "overview";
let loginModeLoaded = false;

function route() {
  const wanted = location.hash.slice(1);
  page = Object.hasOwn(PAGES, wanted) ? wanted : "overview";
  for (const section of document.querySelectorAll(".page")) section.hidden = section.dataset.page !== page;
  for (const link of document.querySelectorAll(".nav a")) {
    if (link.dataset.page === page) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  }
  $("page-title").textContent = PAGES[page][0];
  $("page-lead").textContent = PAGES[page][1];
  document.title = `${PAGES[page][0]} · ${state.config?.server.motd || t("Horizon server")}`;
  if (page === "console") scrollLog(true);
  if (page === "plugins" && !$('app').hidden) refreshPlugins();
  if (state.status) renderUpdate(state.status.update);
  renderCharts();
}

function show(view) {
  $("app").hidden = view !== "app";
  $("signed-out").hidden = view !== "signed-out";
  $("gone").hidden = view !== "gone";
  if (view === "signed-out" && !loginModeLoaded) {
    loginModeLoaded = true;
    api("GET", "/login/mode").then(({ key }) => {
      $("key-login").hidden = !key;
      $("login-link-help").hidden = key;
    }).catch(() => { loginModeLoaded = false; });
  }
}

// --- State ----------------------------------------------------------------------------------------------

const state = {
  status: null,
  samples: [],
  metricsSince: 0,
  logNext: 0,
  lines: [],
  players: null,
  backups: [],
  crashes: [],
  config: null,
};

// --- Status, power and update ---------------------------------------------------------------------------

function renderStatus(status) {
  state.status = status;
  const phase = $("phase");
  phase.textContent = t(`phase.${status.phase}`);
  phase.className = `state ${status.phase}`;
  $("uptime").textContent = status.engine_uptime_seconds != null ? t("\u00b7 up {duration}", { duration: duration(status.engine_uptime_seconds) }) : "";
  $("version").textContent = `horizon-server ${status.version}`;
  const stopped = status.phase === "stopped";
  $("start").hidden = !stopped;
  $("restart").hidden = stopped;
  const crashes = state.crashes.length;
  $("nav-reports").textContent = crashes ? String(crashes) : "";
  renderUpdate(status.update);
  renderOnline();
  renderFacts();
}

// Updates (horizon-server update): a newer release, one under way, or how the last one ended.
function renderUpdate(update) {
  const offer = update && !update.busy && update.available;
  let text = "";
  if (update && update.busy) text = t("Updating: {version}\u2026", { version: update.busy });
  else if (offer) text = t("Horizon {available} is out. This server runs {installed}.", { available: update.available, installed: update.installed });
  else if (update && update.last && !update.ok) text = update.last;
  $("update").hidden = !text || page !== "overview";
  $("update").classList.toggle("failed", Boolean(update && update.last && !update.ok && !update.busy && !offer));
  $("update-text").textContent = text;
  $("update-button").hidden = !offer;
}

function onlinePlayers() {
  const game = state.status && state.status.sample && state.status.sample.game;
  return (game && game.players) || [];
}

function maxPlayers() {
  const game = state.status && state.status.sample && state.status.sample.game;
  return (game && game.max_players) || (state.config && state.config.server.max_players) || null;
}

// --- Charts ---------------------------------------------------------------------------------------------

function sparkline(id, values, { max, line } = {}) {
  const canvas = $(id);
  if (!canvas || !canvas.offsetParent) return;
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth, height = canvas.clientHeight;
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  const g = canvas.getContext("2d");
  g.setTransform(ratio, 0, 0, ratio, 0, 0);
  g.clearRect(0, 0, width, height);
  const css = getComputedStyle(document.documentElement);
  const accent = css.getPropertyValue("--accent-soft").trim();
  const known = values.filter((v) => v != null);
  const top = Math.max(max || 0, ...known, 1e-9) * 1.08;
  const y = (v) => height - 2 - (v / top) * (height - 4);
  if (line != null) {
    g.strokeStyle = css.getPropertyValue("--rule").trim();
    g.lineWidth = 1;
    g.setLineDash([3, 4]);
    g.beginPath();
    g.moveTo(0, y(line));
    g.lineTo(width, y(line));
    g.stroke();
    g.setLineDash([]);
  }
  const span = 600;
  const x = (i) => width - ((values.length - 1 - i) / (span - 1)) * width;
  // Runs of known values, each drawn as a line over a soft fill.
  const runs = [];
  values.forEach((v, i) => {
    if (v == null) return;
    if (!runs.length || values[i - 1] == null) runs.push([]);
    runs[runs.length - 1].push([x(i), y(v)]);
  });
  const gradient = g.createLinearGradient(0, 0, 0, height);
  gradient.addColorStop(0, `color-mix(in srgb, ${accent} 30%, transparent)`);
  gradient.addColorStop(1, "transparent");
  for (const run of runs) {
    g.beginPath();
    run.forEach(([px, py], i) => (i ? g.lineTo(px, py) : g.moveTo(px, py)));
    g.strokeStyle = accent;
    g.lineWidth = 1.6;
    g.lineJoin = "round";
    g.stroke();
    g.lineTo(run[run.length - 1][0], height);
    g.lineTo(run[0][0], height);
    g.closePath();
    g.fillStyle = gradient;
    g.fill();
  }
}

function renderCharts() {
  if (page !== "overview") return;
  const s = state.samples;
  const last = s[s.length - 1] || {};
  const tps = last.tps, mspt = last.mspt;
  $("tps-now").textContent = tps == null ? "–" : tps.toFixed(1);
  $("tps-now").className = tps == null ? "none" : tps < 15 ? "bad" : tps < 19.5 ? "meh" : "";
  $("mspt-now").textContent = mspt == null ? "–" : `${mspt.toFixed(1)} ms`;
  $("mspt-now").className = mspt == null ? "none" : mspt > 50 ? "bad" : mspt > 40 ? "meh" : "";
  $("mspt-note").textContent = last.mspt_p95 != null ? t("95 % of ticks under {p95} ms \u00b7 budget 50 ms", { p95: last.mspt_p95.toFixed(1) }) : t("budget 50 ms");
  const online = onlinePlayers().length, cap = maxPlayers();
  $("players-now").textContent = state.status && state.status.sample && state.status.sample.game ? `${online}${cap ? ` / ${cap}` : ""}` : "–";
  const peak = Math.max(0, ...s.map((x) => x.players || 0));
  $("players-note").textContent = s.length ? t("peak {peak} in the last {minutes} min", { peak, minutes: Math.max(1, Math.round(s.length / 60)) }) : "";
  $("ram-now").textContent = bytes(last.rss_bytes);
  $("cpu-note").textContent = last.cpu_percent == null ? "" : t("CPU {percent} % of a core", { percent: Math.round(last.cpu_percent) });
  sparkline("tps", s.map((x) => x.tps), { max: 20, line: 20 });
  sparkline("mspt", s.map((x) => x.mspt), { max: 50, line: 50 });
  sparkline("players-chart", s.map((x) => x.players), { max: cap || 1 });
  sparkline("ram", s.map((x) => (x.rss_bytes == null ? null : x.rss_bytes / 1e9)));
}

// --- Overview -------------------------------------------------------------------------------------------

function renderOnline() {
  const players = onlinePlayers();
  $("nav-players").textContent = state.status && state.status.sample && state.status.sample.game ? String(players.length) : "";
  fill("overview-players", "overview-no-players", players.slice(0, 6).map((p) => h("li", {}, avatar(p.name), h("span", { class: "row-main" }, h("b", {}, p.name), isOp(p.name) && h("small", {}, t("Operator"))), ping(p.ping_ms))));
  if (page === "players") renderPlayers();
}

function renderFacts() {
  const c = state.config, s = state.status;
  if (!c || !s) return;
  $("server-name").textContent = c.server.motd || t("Horizon server");
  $("server-address").textContent = c.server.bind ? `${c.server.bind}:${c.server.port}` : t("port {port}", { port: c.server.port });
  const last = state.backups.map((b) => backupParts(b.name).when).filter(Boolean).sort((a, b) => b - a)[0];
  const facts = [
    [t("Game"), [t(`mode.${c.world.game_mode}`), t(`difficulty.${c.world.difficulty}`), c.world.hardcore && t("hardcore")].filter(Boolean).join(" · ")],
    [t("World"), c.world.name],
    [t("Who can join"), c.server.whitelist ? t("Whitelisted players") : t("Everyone")],
    [t("Accounts"), c.server.online_mode ? t("Checked with Minecraft") : t("Not checked (offline mode)")],
    [t("Last backup"), last ? t("{ago} \u00b7 every {interval}", { ago: ago(last), interval: c.backups.every }) : t("none yet \u00b7 every {interval}", { interval: c.backups.every })],
    [t("Supervisor up"), duration(s.uptime_seconds)],
  ];
  if (s.crashes) facts.push([t("Crashes"), t("{count} since the supervisor started", { count: s.crashes })]);
  $("facts").replaceChildren(...facts.flatMap(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]));
}

// --- Players --------------------------------------------------------------------------------------------

function isOp(name) {
  return Boolean(state.players && state.players.ops.some((o) => o.name.toLowerCase() === name.toLowerCase()));
}

async function kick(name) {
  const answer = await ask({ title: t("Kick {name}?", { name }), text: t("They can join again straight away."), ok: t("Kick"), danger: true, fields: [{ name: "reason", label: t("Reason (optional)"), placeholder: t("Shown to the player") }] });
  if (answer) await act("POST", "/api/v1/players/kick", { name, reason: answer.reason || undefined }, t("Kicked {name}.", { name }));
}

async function ban(name) {
  const answer = await ask({ title: t("Ban {name}?", { name }), text: t("They are kicked and can't join until you pardon them."), ok: t("Ban"), danger: true, fields: [{ name: "reason", label: t("Reason (optional)"), placeholder: t("Shown to the player") }] });
  if (answer && (await act("POST", "/api/v1/players/ban", { name, reason: answer.reason || undefined }, t("Banned {name}.", { name })))) refreshLists();
}

async function toggleOp(name) {
  const op = isOp(name);
  if (await act("POST", op ? "/api/v1/players/deop" : "/api/v1/players/op", { name }, op ? t("{name} is no longer an operator.", { name }) : t("{name} is now an operator.", { name }))) refreshLists();
}

function renderPlayers() {
  const lists = state.players;
  const online = onlinePlayers();
  fill("players-online", "players-none", online.map((p) => h(
    "li", {},
    avatar(p.name),
    h("span", { class: "row-main" }, h("b", {}, p.name), h("small", {}, isOp(p.name) ? t("Operator") : t("Player"))),
    ping(p.ping_ms),
    h("span", { class: "row-actions" },
      button(isOp(p.name) ? t("Deop") : t("Op"), () => toggleOp(p.name), { kind: "ghost" }),
      button(t("Kick"), () => kick(p.name), { kind: "ghost" }),
      button(t("Ban"), () => ban(p.name), { kind: "destructive" })),
  )));
  if (!lists) return;
  const banRow = (title, detail, tag, pardon) => h(
    "li", {}, h("span", { class: "file-icon" }, icon("user")),
    h("span", { class: "row-main" }, h("b", {}, title), h("small", {}, detail)),
    h("span", { class: "tag danger" }, tag),
    h("span", { class: "row-actions" }, button(t("Pardon"), pardon, { kind: "ghost" })),
  );
  const since = (b) => [b.reason || t("No reason given"), b.created ? t("since {date}", { date: b.created.slice(0, 10) }) : null, b.expires ? t("until {date}", { date: b.expires.slice(0, 10) }) : null].filter(Boolean).join(" · ");
  fill("bans", "bans-none", [
    ...lists.bans.map((b) => banRow(b.name, since(b), t("Player"), async () => {
      if (await act("POST", "/api/v1/players/pardon", { name: b.name }, t("Pardoned {name}.", { name: b.name }))) refreshLists();
    })),
  ]);
}

// --- Console --------------------------------------------------------------------------------------------

const MAX_LINES = 2000;

function kind(line) {
  if (line.startsWith("> ")) return "you";
  if (/\/(ERROR|FATAL)\]/.test(line)) return "error";
  if (/\/WARN\]/.test(line)) return "warn";
  if (/\]: (<[^>]+>|\[[^\]]+\] )/.test(line) && !/\[Supervisor\//.test(line)) return "chat";
  if (/\[Supervisor\//.test(line)) return "sup";
  return "";
}

function lineElement(line) {
  const k = kind(line);
  const m = /^(\[\d\d:\d\d:\d\d\])(.*)$/.exec(line);
  return h("span", { class: `line ${k}` }, m ? [h("span", { class: "t" }, m[1]), m[2]] : line);
}

function atBottom(el) {
  return el.scrollTop + el.clientHeight >= el.scrollHeight - 12;
}

function scrollLog(force) {
  const log = $("log");
  if (force) log.scrollTop = log.scrollHeight;
  $("jump").hidden = atBottom(log);
}

function appendLog(lines) {
  if (!lines.length) return;
  state.lines.push(...lines);
  const trimmed = state.lines.length > MAX_LINES + 100;
  if (trimmed) state.lines.splice(0, state.lines.length - MAX_LINES);
  const log = $("log");
  const stick = atBottom(log);
  if (trimmed) renderLog();
  else log.append(...lines.map(lineElement));
  if (stick) log.scrollTop = log.scrollHeight;
  else $("jump").hidden = false;
  $("log-tail").replaceChildren(...state.lines.slice(-10).map(lineElement));
  $("log-tail").scrollTop = $("log-tail").scrollHeight;
}

function renderLog() {
  $("log").replaceChildren(...state.lines.map(lineElement));
  scrollLog(true);
}

const history = [];
let historyAt = 0;

// --- Backups --------------------------------------------------------------------------------------------

function renderBackups() {
  const list = [...state.backups].sort((a, b) => (backupParts(b.name).when || 0) - (backupParts(a.name).when || 0));
  const c = state.config;
  $("backup-schedule").textContent = c ? (c.backups.every === "0" ? t("Automatic backups are off.") : t("Automatic backup every {interval}, the last {keep} kept.", { interval: c.backups.every, keep: c.backups.keep })) : "";
  fill("backups", "no-backups", list.map((b) => {
    const { when, label } = backupParts(b.name);
    const title = label || (when ? dateFormat.format(when) : b.name);
    const detail = [when && label ? dateFormat.format(when) : null, when ? ago(when) : null].filter(Boolean).join(" · ");
    return h(
      "li", {}, h("span", { class: "file-icon" }, icon("folder-01")),
      h("span", { class: "row-main" }, h("b", {}, title), h("small", {}, detail || b.name)),
      h("span", { class: "row-side" }, bytes(b.bytes)),
      h("span", { class: "row-actions" },
        button(t("Restore"), () => restore(b.name), { kind: "ghost" })),
    );
  }));
}

async function restore(name) {
  const answer = await ask({
    title: t("Restore this backup?"),
    text: t("The server restarts with it; players are disconnected for a moment. The current world is kept in backups/."),
    ok: t("Restore"),
    danger: true,
  });
  if (answer) await act("POST", "/api/v1/restore", { name }, t("Restoring {name}: the server is restarting.", { name }));
}

// --- Crash reports ------- -----------------------------------------------------------------------------

let crashShown = null;

/// "crash-2026-11-03_21.14.07-server.txt" → "Nov 3, 2026, 9:14 PM".
function crashTitle(name) {
  const m = /^crash-(\d{4})-(\d{2})-(\d{2})_(\d{2})\.(\d{2})\.(\d{2})/.exec(name);
  return m ? dateFormat.format(new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6])) : name;
}

function renderCrashes() {
  $("nav-reports").textContent = state.crashes.length ? String(state.crashes.length) : "";
  fill("crashes", "no-crashes", state.crashes.map((r) => {
    const li = h(
      "li", { class: r.name === crashShown ? "selected" : "" }, h("span", { class: "file-icon" }, icon("information-circle")),
      h("span", { class: "row-main" }, h("b", {}, crashTitle(r.name)), h("small", {}, `${r.name} · ${bytes(r.bytes)}`)),
      h("span", { class: "row-actions" }, button(t("Read"), () => readCrash(r.name), { kind: "ghost" })),
    );
    return li;
  }));
}

async function readCrash(name) {
  try {
    const response = await fetch(`/api/v1/crash-reports/read?name=${encodeURIComponent(name)}`, { credentials: "same-origin", headers: { "X-Horizon": "1" } });
    if (response.status === 401) throw new SignedOut();
    if (!response.ok) throw new Error(t((await response.json().catch(() => ({}))).error || "error {status}", { status: response.status }));
    crashShown = name;
    $("crash-name").textContent = name;
    $("crash-text").textContent = await response.text();
    $("crash-viewer").hidden = false;
    renderCrashes();
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    else toast(e.message, true);
  }
}

// --- Settings -------------------------------------------------------------------------------------------

const SERVER_SETTINGS = [
  ["settings.general", [
    ["server.motd", "text", { maxlength: 255 }],
    ["server.max_players", "integer", { min: 1, max: 1000 }],
    ["server.whitelist", "boolean"],
    ["server.enforce_whitelist", "boolean"],
  ]],
  ["settings.world", [
    ["world.difficulty", "choice", { options: ["peaceful", "easy", "normal", "hard"].map((v) => [v, t(`difficulty.${v}`)]) }],
    ["world.game_mode", "choice", { options: ["survival", "creative", "adventure", "spectator"].map((v) => [v, t(`mode.${v}`)]) }],
    ["server.view_distance", "integer", { min: 2, max: 32 }],
    ["server.simulation_distance", "integer", { min: 2, max: 32 }],
  ]],
  ["Backups", [
    ["backups.every", "text", { maxlength: 20 }],
    ["backups.keep", "integer", { min: 1, max: 1000 }],
  ]],
];

function settingsStatus(id, text, error = false) {
  const el = $(id);
  el.textContent = text;
  el.classList.toggle("error", error);
  el.hidden = !text;
}

function configEditor(form, groups, original, statusId, save) {
  const inputs = [];
  const sections = groups.map(([title, fields]) => {
    const labels = fields.filter((field) => Object.hasOwn(original, field.id)).map((field) => {
      const value = original[field.id];
      const options = field.kind === "boolean"
        ? [["true", t("Enabled")], ["false", t("Disabled")]] : field.options;
      const input = options
        ? h("select", { name: field.id }, options.map(([v, label]) => h("option", { value: v }, label)))
        : h("input", { name: field.id, type: field.kind === "integer" ? "number" : "text",
          min: field.min, max: field.max, maxlength: field.maxlength, required: field.kind === "integer" });
      input.value = value;
      const read = () => field.kind === "boolean" ? input.value === "true"
        : field.kind === "integer" ? input.valueAsNumber : input.value;
      inputs.push([field.id, input, read]);
      return h("label", {}, field.label, input, field.description && h("small", {}, field.description));
    });
    return labels.length && h("fieldset", {}, title && h("legend", {}, title), labels);
  }).filter(Boolean);
  const submit = h("button", { type: "submit", class: "primary", disabled: true }, t("Save changes"));
  form.replaceChildren(...sections, ...(inputs.length ? [submit] : []));
  const changes = () => Object.fromEntries(inputs.map(([id, , read]) => [id, read()])
    .filter(([id, value]) => original[id] !== value));
  const refreshSubmit = () => { submit.disabled = !Object.keys(changes()).length; };
  form.oninput = refreshSubmit;
  form.onsubmit = async (event) => {
    event.preventDefault();
    if (submit.disabled) return;
    const update = changes();
    submit.disabled = true;
    for (const [, input] of inputs) input.disabled = true;
    settingsStatus(statusId, t("Saving…"));
    try {
      const message = await save(update);
      Object.assign(original, update);
      if (submit.isConnected) settingsStatus(statusId, message);
    } catch (e) {
      if (e instanceof SignedOut) show("signed-out");
      else if (submit.isConnected) settingsStatus(statusId, e.message, true);
    } finally {
      for (const [, input] of inputs) input.disabled = false;
      refreshSubmit();
    }
  };
  return inputs.length > 0;
}

function renderServerSettings() {
  const form = $("server-settings");
  if (form.childElementCount) return;
  const values = {};
  const groups = SERVER_SETTINGS.map(([section, fields]) => [t(section), fields.map(([id, kind, options]) => {
    const [group, key] = id.split(".");
    const value = state.config[group]?.[key];
    if (value != null) values[id] = value;
    return { id, kind, label: t(id), description: t(`${id}.help`), ...options };
  })]);
  configEditor(form, groups, values, "settings-status", async (changes) => {
    const result = await api("PATCH", "/api/v1/config", { changes });
    await loadConfig();
    const applied = result.applied.map((id) => t(id)).join("、");
    const restart = result.restart.map((id) => t(id)).join("、");
    if (!result.running) return t("settings.nextStart");
    return [applied && t("settings.applied", { applied }), restart && t("settings.restart", { restart })]
      .filter(Boolean).join(" ") || t("settings.saved");
  });
  settingsStatus("settings-status", "");
}

// --- Plugins --------------------------------------------------------------------------------------------

let pluginKey;
let readingPlugins = false;
async function refreshPlugins() {
  if (readingPlugins) return;
  readingPlugins = true;
  const status = $("plugin-status");
  try {
    const { plugins } = await api("GET", "/api/v1/panel/plugins");
    const key = JSON.stringify(plugins);
    if (key !== pluginKey) {
      pluginKey = key;
      fill("plugins", null, plugins.map(({ id, version, card }) => h("li", {},
        h("span", { class: "file-icon", "aria-hidden": "true" }, icon("puzzle")),
        h("span", { class: "row-main" },
          h("b", {}, card ? card.title : id),
          card && h("small", {}, card.description ? `${id} · ${card.description}` : id),
        ),
        h("span", { class: "row-side" }, version),
        card && h("span", { class: "row-actions" },
          card.config && button(t("Edit settings"), () => openPluginSettings(id, card), { kind: "primary" }),
          card.actions.map((action) => button(action.label, () => pluginAction(id, card.title, action), { kind: "ghost" })),
        ),
      )));
    }
    status.hidden = plugins.length > 0;
    if (!status.hidden) status.textContent = t("No plugins loaded.");
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    pluginKey = undefined;
    $("plugins").replaceChildren();
    status.textContent = e.message;
    status.hidden = false;
  } finally {
    readingPlugins = false;
  }
}

let pluginEditorRequest = 0;
async function openPluginSettings(id, card) {
  const request = ++pluginEditorRequest;
  const path = `/api/v1/panel/plugins/config?plugin=${encodeURIComponent(id)}`;
  const editor = $("plugin-settings");
  const form = $("plugin-settings-form");
  $("plugin-settings-title").textContent = t("plugin.settings", { title: card.title });
  $("plugin-settings-note").textContent = card.config.note;
  form.replaceChildren();
  editor.hidden = false;
  settingsStatus("plugin-settings-status", t("Loading settings…"));
  editor.scrollIntoView({ block: "nearest" });
  try {
    let { revision, values } = await api("GET", path);
    if (request !== pluginEditorRequest) return;
    const available = configEditor(form, [["", card.config.fields]], values, "plugin-settings-status", async (changes) => {
      ({ revision } = await api("PATCH", path, { revision, changes }));
      return t("settings.saved");
    });
    settingsStatus("plugin-settings-status", available ? "" : t("plugin.noEditable"));
    if (available) form.querySelector("input, select").focus({ preventScroll: true });
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    else if (request === pluginEditorRequest) settingsStatus("plugin-settings-status", e.message, true);
  }
}

async function pluginAction(plugin, title, action) {
  if (!await ask({ title: t("{action}?", { action: action.label }), text: title, ok: t("Run") })) return;
  const output = $("plugin-output");
  try {
    const reply = await api("POST", "/api/v1/panel/plugins/action", { plugin, action: action.id });
    output.textContent = (reply.output || []).join("\n") || t("Done.");
    output.hidden = false;
  } catch (e) {
    output.hidden = true;
    if (e instanceof SignedOut) show("signed-out");
    else toast(e.message, true);
  }
}

// --- Polling --------------------------------------------------------------------------------------------

async function refreshLists() {
  try {
    const [players, backups, crashes] = await Promise.all([
      api("GET", "/api/v1/players"),
      api("GET", "/api/v1/backups"),
      api("GET", "/api/v1/crash-reports"),
    ]);
    state.players = players;
    state.crashes = crashes.reports || [];
    const key = JSON.stringify(backups.backups);
    if (key !== JSON.stringify(state.backups)) {
      state.backups = backups.backups;
      renderBackups();
    }
    renderPlayers();
    renderOnline();
    renderCrashes();
    renderFacts();
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
  }
}

async function loadConfig() {
  try {
    state.config = (await api("GET", "/api/v1/config")).config;
    renderFacts();
    renderBackups();
    renderServerSettings();
    route();
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    else if (!state.config) settingsStatus("settings-status", e.message, true);
  }
}

let ticks = 0;
let wasDown = false;
async function poll() {
  try {
    const [status, metrics, logs] = await Promise.all([
      api("GET", "/api/v1/status"),
      api("GET", `/api/v1/metrics?since=${state.metricsSince}`),
      api("GET", `/api/v1/logs?since=${state.logNext}`),
    ]);
    show("app");
    if (wasDown || !state.config) {
      wasDown = false;
      await loadConfig();
    }
    for (const s of metrics.samples) {
      state.samples.push(s);
      state.metricsSince = s.t;
    }
    state.samples.splice(0, Math.max(0, state.samples.length - 600));
    renderStatus(status);
    renderCharts();
    state.logNext = logs.next;
    appendLog(logs.lines);
    if (ticks++ % 5 === 0) {
      await refreshLists();
      if (page === "plugins") refreshPlugins();
    }
    if (ticks % 30 === 0) loadConfig();
  } catch (e) {
    if (e instanceof SignedOut) show("signed-out");
    else {
      wasDown = true;
      show(state.status ? "app" : "gone");
      if (state.status) {
        $("phase").textContent = t("not answering");
        $("phase").className = "state down";
      }
    }
  }
}

// --- Wiring ---------------------------------------------------------------------------------------------

function setUp() {
  localizeDocument();
  window.addEventListener("hashchange", route);
  window.addEventListener("resize", renderCharts);
  $("plugin-settings-close").addEventListener("click", () => {
    pluginEditorRequest++;
    $("plugin-settings-form").replaceChildren();
    $("plugin-settings").hidden = true;
  });

  $("start").addEventListener("click", () => act("POST", "/api/v1/start", undefined, t("Starting…")));
  $("restart").addEventListener("click", async () => {
    if (await ask({ title: t("Restart the game?"), text: t("Players are disconnected for a moment and can join again once it's up."), ok: t("Restart") })) {
      await act("POST", "/api/v1/restart", undefined, t("Restarting…"));
    }
  });
  $("stop").addEventListener("click", async () => {
    if (await ask({ title: t("Stop the server?"), text: t("The game saves and stops, and this panel goes with it. Start it again on the server with horizon-server start."), ok: t("Stop"), danger: true })) {
      await act("POST", "/api/v1/stop", undefined, t("Stopping…"));
    }
  });
  $("update-button").addEventListener("click", async () => {
    const update = state.status && state.status.update;
    if (!update || !update.available) return;
    if (await ask({ title: t("Update to {version}?", { version: update.available }), text: t("The world is backed up, then the game restarts on the new release. If it doesn't come up, the server goes back to {version} by itself.", { version: update.installed }), ok: t("Update") })) {
      await act("POST", "/api/v1/update", {}, t("Updating…"));
    }
  });
  $("sign-out").addEventListener("click", async () => {
    await fetch("/logout", { method: "POST", credentials: "same-origin", headers: { "X-Horizon": "1" } }).catch(() => {});
    show("signed-out");
  });
  $("key-login").addEventListener("submit", async (event) => {
    event.preventDefault();
    const input = $("access-key");
    const submit = $("login-submit");
    const error = $("login-error");
    input.disabled = submit.disabled = true;
    error.hidden = true;
    try {
      await api("POST", "/login", { key: input.value });
      location.reload();
    } catch (e) {
      error.textContent = e.message;
      error.hidden = false;
    } finally {
      input.value = "";
      input.disabled = submit.disabled = false;
      if (!error.hidden) input.focus();
    }
  });

  // Players.
  $("ban-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const target = event.target.target.value.trim();
    const reason = event.target.reason.value.trim() || undefined;
    const reply = await act("POST", "/api/v1/players/ban", { name: target, reason }, t("Banned {name}.", { name: target }));
    if (reply) {
      event.target.reset();
      refreshLists();
    }
  });

  // Console.
  $("log").addEventListener("scroll", () => scrollLog(false));
  $("jump").addEventListener("click", () => scrollLog(true));
  $("command-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const line = $("command").value.trim().replace(/^\//, "");
    if (!line) return;
    $("command").value = "";
    history.push(line);
    historyAt = history.length;
    appendLog([`> ${line}`]);
    scrollLog(true);
    try {
      const reply = await api("POST", "/api/v1/command", { line });
      const output = (reply.output || []).join("\n");
      if (output) appendLog(output.split("\n").map((l) => `  ${l}`));
    } catch (e) {
      toast(e.message, true);
    }
  });
  $("command").addEventListener("keydown", (event) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    historyAt = Math.max(0, Math.min(history.length, historyAt + (event.key === "ArrowUp" ? -1 : 1)));
    $("command").value = history[historyAt] || "";
  });

  // Backups.
  $("backup-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const name = $("backup-name").value.trim();
    toast(t("Backing up…"));
    const made = await act("POST", "/api/v1/backups", name ? { name } : {}, t("Backed up."));
    if (made) {
      toast(t("Backed up to backups/{name}", { name: made.name }));
      $("backup-name").value = "";
      refreshLists();
    }
  });

  // Crash reports.
  $("crash-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("crash-text").textContent);
      toast(t("Copied the report."));
    } catch (e) {
      toast(t("Couldn't copy: select the text instead."), true);
    }
  });

  route();
  poll();
  setInterval(poll, 1000);
}

document.addEventListener("DOMContentLoaded", setUp);
