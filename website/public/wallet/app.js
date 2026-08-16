import init, {
  create_wallet,
  restore_wallet,
  wallet_address,
  wallet_scan_from,
  wallet_phrase,
  wallet_view_key,
  wallet_info,
  reset_scan,
  address_qr_svg,
  ingest_page,
  wallet_balance,
  wallet_history,
  build_send,
  probe_crypto,
} from "./pkg/nightfall_web.js";

const STORE = "nf-web-wallet-v1";
const NODE_STORE = "nf-web-node";
const HIDE_STORE = "nf-web-hide";
const $ = (s, r = document) => r.querySelector(s);
const app = $("#app");

const WARN =
  "This phone or browser trusts a node for what it shows. A hostile node can hide a payment or invent one on the screen. It cannot spend — the seed never leaves this device. Anyone who can run script on this page can read a saved wallet. The 24 words are the real backup.";

const FEE = "0.001";

let wasmReady = init();
let state = null;
let phrasePending = null;
let tab = "wallet";
let sheet = null;
let hideBal = localStorage.getItem(HIDE_STORE) === "1";
let lastBal = null;
let lastHist = [];
let lastTip = 0;
let lastStatus = "";
let lastErr = "";

function save() {
  if (state) localStorage.setItem(STORE, state);
}
function loadSaved() {
  return localStorage.getItem(STORE);
}
function forget() {
  localStorage.removeItem(STORE);
  state = null;
  lastBal = null;
  lastHist = [];
}

function nodeUrl() {
  return localStorage.getItem(NODE_STORE) || "";
}

async function rpc(method, params = {}) {
  const r = await fetch("/wallet-api", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ method, params, id: 1 }),
  });
  const j = await r.json();
  if (j.error) throw new Error(typeof j.error === "string" ? j.error : JSON.stringify(j.error));
  return j.result;
}

function parseJson(s) {
  return typeof s === "string" ? JSON.parse(s) : s;
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function wasmCall(fn, ...args) {
  try {
    return fn(...args);
  } catch (e) {
    const m = String(e && e.message ? e.message : e);
    if (/unreachable|Unreachable/i.test(m)) {
      throw new Error(
        "Could not build the transaction in this browser. Try again, or send from Core / the Android app. (" +
          m +
          ")",
      );
    }
    throw e instanceof Error ? e : new Error(m);
  }
}

function fmtAmt(s) {
  if (!s && s !== 0) return "—";
  const [w, f = ""] = String(s).split(".");
  const whole = w.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  if (!f || /^0+$/.test(f)) return whole + ".00";
  return whole + "." + f.replace(/0+$/, "");
}

function shortAddr(a) {
  if (!a || a.length < 16) return a || "";
  return a.slice(0, 8) + "…" + a.slice(-6);
}

function when(ts, height, pending) {
  if (pending) return "pending";
  if (ts) {
    const d = new Date(ts * 1000);
    if (!Number.isNaN(d.getTime()) && ts > 1_000_000_000) {
      return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
    }
  }
  return height != null ? `#${height}` : "";
}

function dirMeta(d) {
  if (d === "Received" || d === "Mined") return { label: d, cls: "in", sign: "+", icon: "↓" };
  return { label: d || "Sent", cls: "out", sign: "−", icon: "↑" };
}

function screen(html) {
  app.innerHTML = html;
}

function icons() {
  return {
    wallet: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><rect x="3" y="6" width="18" height="13" rx="3"/><path d="M16 12h4"/></svg>`,
    activity: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M4 19V5M10 19V9M16 19v-7M22 19V3"/></svg>`,
    settings: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9c.3.6.9 1 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>`,
  };
}

function nav() {
  const i = icons();
  return `<nav class="nav">
    <button data-tab="wallet" class="${tab === "wallet" ? "on" : ""}">${i.wallet}Wallet</button>
    <button data-tab="activity" class="${tab === "activity" ? "on" : ""}">${i.activity}Activity</button>
    <button data-tab="settings" class="${tab === "settings" ? "on" : ""}">${i.settings}Settings</button>
  </nav>`;
}

function bindNav() {
  document.querySelectorAll(".nav [data-tab]").forEach((b) => {
    b.onclick = () => {
      tab = b.dataset.tab;
      sheet = null;
      renderApp();
    };
  });
}

function onboard() {
  screen(`
    <div class="onboard">
      <img class="logo" src="/assets/logo-256.png" alt="">
      <h1>NIGHTFALLCOIN</h1>
      <p class="lede">Receive and send without installing an app. Add this page to your home screen.</p>
      <div class="card"><p class="hint">${WARN}</p></div>
      <button class="primary" id="create">Create a wallet</button>
      <button class="ghost" id="restore-toggle">I have 24 words</button>
      <div id="restore" hidden>
        <textarea id="words" placeholder="twenty four words…"></textarea>
        <input id="birth" placeholder="Birth height (0 if unsure)" value="0" inputmode="numeric">
        <p class="hint">A number that is too high silently misses coins. Too low only costs time.</p>
        <button class="primary" id="do-restore">Restore</button>
      </div>
      <p id="err" class="warn"></p>
      <p class="hint"><a href="/">Back to nightfallcoin.org</a></p>
    </div>
  `);
  $("#create").onclick = onCreate;
  $("#restore-toggle").onclick = () => {
    $("#restore").hidden = !$("#restore").hidden;
  };
  $("#do-restore").onclick = onRestore;
}

function backup(phrase) {
  const words = phrase.trim().split(/\s+/);
  screen(`
    <div class="onboard">
      <h1>Write these 24 words down</h1>
      <p class="hint">On paper. Not a screenshot, not the cloud. Nobody can reset this.</p>
      <div class="card mono">${escapeHtml(phrase)}</div>
      <p class="hint">Type word 4 and word 18 to continue.</p>
      <input id="w4" placeholder="Word 4" autocomplete="off">
      <input id="w18" placeholder="Word 18" autocomplete="off">
      <button class="primary" id="done" disabled>I have the words</button>
      <p id="err" class="warn"></p>
    </div>
  `);
  const check = () => {
    const ok =
      $("#w4").value.trim().toLowerCase() === (words[3] || "") &&
      $("#w18").value.trim().toLowerCase() === (words[17] || "");
    $("#done").disabled = !ok;
  };
  $("#w4").oninput = check;
  $("#w18").oninput = check;
  $("#done").onclick = () => {
    phrasePending = null;
    save();
    tab = "wallet";
    renderApp();
    sync();
  };
}

function txRows(rows, limit) {
  const list = limit ? rows.slice(0, limit) : rows;
  if (!list.length) return `<p class="dim">No movements yet.</p>`;
  return list
    .map((e) => {
      const m = dirMeta(e.direction);
      return `<div class="tx">
        <div class="badge">${m.icon}</div>
        <div class="mid">
          <strong>${escapeHtml(m.label)}</strong>
          <div class="dim">${escapeHtml(when(e.timestamp, e.height, e.pending))}${
            e.memo ? " · " + escapeHtml(e.memo) : ""
          }</div>
        </div>
        <div class="amt ${m.cls}">${m.sign}${fmtAmt(e.amount)} NIGHT</div>
      </div>`;
    })
    .join("");
}

function renderApp() {
  if (!state) {
    onboard();
    return;
  }
  if (sheet === "receive") return renderReceive();
  if (sheet === "send") return renderSend();
  if (sheet === "seed") return renderSeed();
  if (sheet === "viewkey") return renderViewKey();
  if (tab === "activity") return renderActivity();
  if (tab === "settings") return renderSettings();
  renderHome();
}

function renderHome() {
  const total = hideBal ? "••••••" : fmtAmt(lastBal?.total);
  const spend = hideBal ? "••••" : fmtAmt(lastBal?.available);
  const unlock = hideBal ? "••••" : fmtAmt(lastBal?.immature);
  const status = lastErr ? `<span class="warn">${escapeHtml(lastErr)}</span>` : escapeHtml(lastStatus || "tap to sync");
  screen(`
    <header class="sky">
      <div class="topbar">
        <button class="iconbtn" id="to-settings" aria-label="Settings">☰</button>
        <button class="iconbtn" id="do-sync" aria-label="Sync">↻</button>
      </div>
      <div class="brand-wrap">
        <img src="/assets/logo-256.png" alt="">
        <p class="word">NIGHTFALLCOIN</p>
      </div>
      <div class="mountains"></div>
    </header>
    <div class="wrap">
      <section class="glass">
        <div class="row">
          <span class="kicker">Total balance <button class="linkish" id="hide">${hideBal ? "show" : "hide"}</button></span>
          <span class="pill"><span class="dot"></span> Nightfall</span>
        </div>
        <div class="balance">
          <span class="num">${total}</span>
          <span class="tick">NIGHT</span>
        </div>
        <p class="sub">spendable ${spend} · unlocking ${unlock}</p>
        <div class="actions">
          <button class="recv" id="recv">↓ Receive</button>
          <button class="send" id="send">↗ Send</button>
        </div>
      </section>
      <section class="section glass" style="margin-top:14px">
        <div class="row"><h2>Network</h2></div>
        <div class="stat"><span class="dim">Tip</span><span>${lastTip || lastBal?.tip || "—"}</span></div>
        <div class="stat"><span class="dim">Scanned to</span><span>${lastBal?.scanned_to ?? "—"}</span></div>
        <div class="stat"><span class="dim">Fee</span><span>${FEE} NIGHT · burned while subsidy lasts</span></div>
        <p class="status-line" id="status">${status}</p>
      </section>
      <section class="section">
        <div class="row">
          <h2>Recent</h2>
          <button class="linkish" id="all-tx">See all</button>
        </div>
        <div class="glass">${txRows(lastHist, 5)}</div>
      </section>
    </div>
    ${nav()}
  `);
  bindNav();
  $("#hide").onclick = () => {
    hideBal = !hideBal;
    localStorage.setItem(HIDE_STORE, hideBal ? "1" : "0");
    renderHome();
  };
  $("#recv").onclick = () => {
    sheet = "receive";
    renderApp();
  };
  $("#send").onclick = () => {
    sheet = "send";
    renderApp();
  };
  $("#all-tx").onclick = () => {
    tab = "activity";
    renderApp();
  };
  $("#to-settings").onclick = () => {
    tab = "settings";
    renderApp();
  };
  $("#do-sync").onclick = () => sync();
}

function renderActivity() {
  screen(`
    <div class="screen">
      <p class="kicker">Activity</p>
      <h1>Movements</h1>
      <p class="hint">Heights come from the node this browser trusts.</p>
      <div class="glass">${txRows(lastHist)}</div>
    </div>
    ${nav()}
  `);
  bindNav();
}

function renderSettings() {
  let info = { birth_height: "—", scanned_to: "—", outputs: "—" };
  try {
    info = parseJson(wallet_info(state));
  } catch (_) {}
  screen(`
    <div class="screen">
      <p class="kicker">Settings</p>
      <h1>Wallet</h1>
      <button class="settings-item" id="show-seed">
        Recovery phrase
        <small>24 words. Anyone who sees them can spend.</small>
      </button>
      <button class="settings-item" id="show-view">
        View key
        <small>Reads amounts and memos. Cannot spend.</small>
      </button>
      <button class="settings-item" id="rescan">
        Rescan from birth height
        <small>Birth ${info.birth_height} · scanned to ${info.scanned_to} · ${info.outputs} unspent</small>
      </button>
      <div class="card">
        <p class="kicker">Trusted node</p>
        <p class="hint">The website Worker proxies to the seed. This field is informational — the browser always talks to /wallet-api.</p>
        <input id="node" value="${escapeHtml(nodeUrl() || "seed.nightfallcoin.org (via /wallet-api)")}" readonly>
        <p class="hint">Tip ${lastTip || lastBal?.tip || "—"} · protocol v8 · wallet 0.7.0</p>
      </div>
      <div class="card"><p class="hint">${WARN}</p></div>
      <button class="ghost" id="wipe">Remove wallet from this browser</button>
      <p class="hint" style="margin-top:1rem"><a href="/">nightfallcoin.org</a></p>
    </div>
    ${nav()}
  `);
  bindNav();
  $("#show-seed").onclick = () => {
    sheet = "seed";
    renderApp();
  };
  $("#show-view").onclick = () => {
    sheet = "viewkey";
    renderApp();
  };
  $("#rescan").onclick = async () => {
    if (!confirm("Rescan from the birth height? This forgets discovered outputs and walks the chain again.")) return;
    try {
      const res = parseJson(reset_scan(state));
      state = res.state;
      save();
      lastStatus = "rescanning…";
      tab = "wallet";
      await sync();
    } catch (e) {
      lastErr = e.message || String(e);
      renderApp();
    }
  };
  $("#wipe").onclick = () => {
    if (confirm("This removes the wallet from this browser only. You need the 24 words to get it back.")) {
      forget();
      onboard();
    }
  };
}

function renderSeed() {
  let phrase = "";
  let err = "";
  try {
    phrase = wallet_phrase(state);
  } catch (e) {
    err = e.message || String(e);
  }
  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Settings</button>
      <h1>Recovery phrase</h1>
      <p class="hint">On paper. Not a screenshot. These 24 words are the wallet.</p>
      <div class="secret mono" id="secret" hidden>${escapeHtml(phrase)}</div>
      <button class="primary" id="reveal">Show the 24 words</button>
      <button class="ghost" id="copy" hidden>Copy phrase</button>
      <p class="warn">${escapeHtml(err)}</p>
    </div>
  `);
  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#reveal").onclick = () => {
    $("#secret").hidden = false;
    $("#copy").hidden = false;
    $("#reveal").hidden = true;
  };
  $("#copy").onclick = async () => {
    await navigator.clipboard.writeText(phrase);
    lastStatus = "phrase copied — clear the clipboard when you can";
  };
}

function renderViewKey() {
  let key = "";
  let err = "";
  try {
    key = wallet_view_key(state);
  } catch (e) {
    err = e.message || String(e);
  }
  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Settings</button>
      <h1>View key</h1>
      <p class="hint">A view key decrypts amounts and memos. It cannot sign. Treat it as sensitive.</p>
      <div class="secret mono">${escapeHtml(key)}</div>
      <button class="primary" id="copy">Copy view key</button>
      <p class="warn">${escapeHtml(err)}</p>
    </div>
  `);
  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#copy").onclick = async () => {
    await navigator.clipboard.writeText(key);
  };
}

function renderReceive() {
  let address = "";
  let qr = "";
  try {
    address = wallet_address(state);
    qr = address_qr_svg(address);
  } catch (e) {
    lastErr = e.message || String(e);
  }
  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Wallet</button>
      <h1>Receive</h1>
      <p class="hint">Share this address. Receiving works while this device is off.</p>
      ${qr ? `<div class="qrbox">${qr}</div>` : ""}
      <div class="card">
        <p class="kicker">Your address</p>
        <p class="mono addrbox">${escapeHtml(address)}</p>
      </div>
      <button class="primary" id="copy">Copy address</button>
      <p class="hint">${escapeHtml(shortAddr(address))}</p>
    </div>
  `);
  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#copy").onclick = async () => {
    await navigator.clipboard.writeText(address);
    lastStatus = "address copied";
    const b = $("#copy");
    if (b) b.textContent = "Copied";
  };
}

function renderSend() {
  const spend = fmtAmt(lastBal?.available);
  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Wallet</button>
      <h1>Send</h1>
      <p class="hint">Wrong address = gone forever. Fee ${FEE} NIGHT, burned while blocks still pay a subsidy. Spendable ${spend} NIGHT.</p>
      <input id="to" placeholder="nf1…" spellcheck="false" autocomplete="off">
      <input id="amt" placeholder="Amount" inputmode="decimal">
      <input id="memo" placeholder="Memo (optional)">
      <button class="primary" id="go">Review</button>
      <p id="err" class="warn"></p>
    </div>
  `);
  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#go").onclick = () => {
    const to = $("#to").value.trim();
    const amt = $("#amt").value.trim();
    const memo = $("#memo").value;
    if (!to.startsWith("nf1") || to.length < 20) {
      $("#err").textContent = "that does not look like an nf1 address";
      return;
    }
    if (!amt) {
      $("#err").textContent = "enter an amount";
      return;
    }
    if (!confirm(`Send ${amt} NIGHT to\n${shortAddr(to)}\nplus ${FEE} NIGHT fee?`)) return;
    doSend(to, amt, memo);
  };
}

async function doSend(to, amt, memo) {
  lastErr = "";
  lastStatus = "checking this browser…";
  tab = "wallet";
  sheet = null;
  renderHome();
  try {
    const probe = wasmCall(probe_crypto);
    lastStatus = "browser ok (" + probe + "). fetching tip…";
    renderHome();
    const tipPage = await rpc("status", {});
    const tip = Number(tipPage.tip_height ?? 0) || 0;
    lastTip = tip;
    lastStatus = "proving range proofs…";
    renderHome();
    await new Promise((r) => setTimeout(r, 40));
    const built = parseJson(wasmCall(build_send, state, to, amt, memo, tip));
    lastStatus = "broadcasting…";
    renderHome();
    const res = await rpc("submit_tx", { tx: built.tx });
    state = built.state;
    save();
    lastStatus = "sent " + (res.txid || built.txid);
    await sync();
  } catch (e) {
    lastErr = e.message || String(e);
    renderHome();
  }
}

async function sync() {
  lastErr = "";
  lastStatus = "syncing…";
  if (tab === "wallet" && !sheet) renderHome();
  try {
    const tipPage = await rpc("status", {});
    const tip = Number(tipPage.tip_height ?? Math.max(0, (tipPage.blocks || 1) - 1)) || 0;
    lastTip = tip;
    let from = wallet_scan_from(state);
    let found = 0;
    for (let i = 0; i < 200; i++) {
      const page = await rpc("scan_feed", { from, limit: 256 });
      const outputs = JSON.stringify(page.outputs || []);
      const spent = JSON.stringify(page.spent || []);
      const scannedTo = page.scanned_to ?? from;
      const res = parseJson(ingest_page(state, outputs, spent, scannedTo));
      state = res.state;
      found += res.found || 0;
      const n = page.blocks || 0;
      if (!n) break;
      const next = (scannedTo || from) + 1;
      if (next <= from) break;
      from = next;
    }
    save();
    lastBal = parseJson(wallet_balance(state, tip));
    lastHist = parseJson(wallet_history(state));
    lastStatus = found ? `found ${found} new output(s)` : "up to date";
  } catch (e) {
    lastErr = e.message || String(e);
  }
  renderApp();
}

async function onCreate() {
  try {
    await wasmReady;
    const tip = await rpc("status", {})
      .then((s) => Number(s.tip_height ?? 0) || 0)
      .catch(() => 0);
    const res = parseJson(create_wallet(tip));
    state = res.state;
    phrasePending = res.phrase;
    backup(res.phrase);
  } catch (e) {
    $("#err").textContent = e.message || String(e);
  }
}

async function onRestore() {
  try {
    await wasmReady;
    const words = $("#words").value.trim();
    const birth = Number($("#birth").value) || 0;
    const res = parseJson(restore_wallet(words, birth));
    state = res.state;
    save();
    tab = "wallet";
    renderApp();
    await sync();
  } catch (e) {
    $("#err").textContent = e.message || String(e);
  }
}

async function boot() {
  try {
    await wasmReady;
    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.register("./sw.js").catch(() => {});
    }
    const saved = loadSaved();
    if (saved) {
      state = saved;
      renderApp();
      await sync();
    } else {
      onboard();
    }
  } catch (e) {
    screen(`<p class="warn" style="padding:2rem">Could not start the wallet: ${escapeHtml(e.message || e)}</p>`);
  }
}

boot();
