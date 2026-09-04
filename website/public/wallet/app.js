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
} from "./pkg/nightfall_web.js?v=090";

const STORE = "nf-web-wallet-v1";
const NODE_STORE = "nf-web-node";
const HIDE_STORE = "nf-web-hide";
const BOOK_STORE = "nf-web-book";
const OUTBOX_STORE = "nf-web-outbox";
const $ = (s, r = document) => r.querySelector(s);
const app = $("#app");

const WARN =
  "This phone or browser trusts a node for what it shows. A hostile node can hide a payment or invent one on the screen. It cannot spend — the seed never leaves this device. Anyone who can run script on this page can read a saved wallet. The 24 words are the real backup.";

const FEE = "0.001";
const BUILD = "0.9.1";

let wasmReady = init();
let state = null;
let phrasePending = null;
let tab = "wallet";
let sheet = null;
let bookFrom = "send";
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

/* Address book.
 *
 * Local to this browser and nothing else. It is never sent anywhere, it is not
 * part of the wallet file, and it does not survive clearing site data — which
 * is the honest trade for not having a server hold a list of who you pay.
 */
function loadBook() {
  try {
    const v = JSON.parse(localStorage.getItem(BOOK_STORE) || "[]");
    return Array.isArray(v) ? v.filter((e) => e && e.addr) : [];
  } catch (_) {
    return [];
  }
}

function saveBook(list) {
  localStorage.setItem(BOOK_STORE, JSON.stringify(list));
}

function bookLabel(addr) {
  const hit = loadBook().find((e) => e.addr === addr);
  return hit ? hit.label : "";
}

function addToBook(label, addr) {
  const list = loadBook().filter((e) => e.addr !== addr);
  list.push({ label: label.trim().slice(0, 40) || shortAddr(addr), addr });
  list.sort((a, b) => a.label.localeCompare(b.label));
  saveBook(list);
}

/* Outbox: transactions this browser broadcast that no block has taken yet.
 *
 * A payment goes to exactly one randomly chosen peer, which is what keeps it
 * from being traced back here, and nothing repeats it. One dropped hop used to
 * end the payment silently. Nodes now forget an unmined transaction after six
 * hours, so the sender is the only one who can put it back — which means the
 * sender has to keep it.
 */
function loadOutbox() {
  try {
    const v = JSON.parse(localStorage.getItem(OUTBOX_STORE) || "[]");
    return Array.isArray(v) ? v : [];
  } catch (_) {
    return [];
  }
}

function saveOutbox(list) {
  localStorage.setItem(OUTBOX_STORE, JSON.stringify(list.slice(-20)));
}

function rememberOutbound(txid, tx) {
  const list = loadOutbox().filter((e) => e.txid !== txid);
  list.push({ txid, tx, at: Math.floor(Date.now() / 1000) });
  saveOutbox(list);
}

/* Re-submit anything still pending, then drop what the wallet now shows as
 * confirmed. A rejection is ignored on purpose: the usual reason is that the
 * inputs are already spent, which is what confirmation looks like from the
 * outside. The history is the authority, not the reply. */
async function flushOutbox() {
  const list = loadOutbox();
  if (!list.length) return;
  const pending = new Set(
    (lastHist || []).filter((e) => e.direction === "Sent" && e.pending).map((e) => e.txid),
  );
  const keep = [];
  for (const e of list) {
    // Give up after a day. By then it is not a dropped hop, it is a payment
    // the network refuses, and retrying for ever hides that.
    const stale = Math.floor(Date.now() / 1000) - (e.at || 0) > 86400;
    if (!pending.has(e.txid) || stale) continue;
    try {
      await rpc("submit_tx", { tx: e.tx });
    } catch (_) {
      /* see above */
    }
    keep.push(e);
  }
  saveOutbox(keep);
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
        "Could not build the transaction in this browser. Try again, or send from the Core wallet. (" +
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
  // The tab bar is written at the end of each template, but it must not live
  // inside the part that scrolls. On iOS a `position: fixed` bar drifts upward
  // with momentum scrolling — it is positioned against a layout viewport that
  // the collapsing toolbar keeps resizing. Lifting it out of the scroller and
  // making it a flex sibling removes the whole class of problem: it is not in
  // the scrolling box, so scrolling cannot move it.
  app.innerHTML = `<div class="view">${html}</div>`;
  const bar = app.querySelector(".view > .nav");
  if (bar) app.appendChild(bar);
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

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (_) {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.left = "-9999px";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  }
}

function wordGrid(phrase) {
  return `<div class="words">${phrase
    .trim()
    .split(/\s+/)
    .map((w, i) => `<span><i>${i + 1}</i>${escapeHtml(w)}</span>`)
    .join("")}</div>`;
}

function backup(phrase) {
  const words = phrase.trim().split(/\s+/);
  screen(`
    <div class="onboard onboard-wide">
      <p class="step">Step 1 of 2 · Back up</p>
      <h1>Write these 24 words down</h1>
      <p class="hint">These words <em>are</em> the wallet. Paper first; a password manager is the next best thing. Not a screenshot, not a chat message, not email.</p>
      ${wordGrid(phrase)}
      <button class="ghost" id="copy">Copy all 24 words</button>
      <p class="ok" id="copied" hidden>Copied. Clear the clipboard when you have stored them.</p>
      <div class="callout">
        <span class="ico" aria-hidden="true">!</span>
        <span><b>Nobody can reset this.</b> There is no company, no support address and no recovery link. Anyone who reads these words can spend your coins; if you lose them, the coins are gone.</span>
      </div>
      <p class="hint" style="margin-top:18px">Two of them, to prove the copy is right.</p>
      <div class="confirm">
        <div>
          <label for="w4">Word 4</label>
          <input id="w4" autocomplete="off" autocapitalize="none" spellcheck="false" inputmode="text">
        </div>
        <div>
          <label for="w18">Word 18</label>
          <input id="w18" autocomplete="off" autocapitalize="none" spellcheck="false" inputmode="text">
        </div>
      </div>
      <button class="primary" id="done" disabled>I have written them down</button>
      <p id="err" class="warn"></p>
    </div>
  `);
  $("#copy").onclick = async () => {
    const ok = await copyText(phrase);
    const n = $("#copied");
    if (n) {
      n.hidden = !ok;
      n.textContent = ok
        ? "Copied. Clear the clipboard when you have stored them."
        : "Could not copy. Select the words and copy them yourself.";
      n.className = ok ? "ok" : "warn";
    }
  };
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
  if (sheet === "book") return renderBook();
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
        <div class="stat"><span class="dim">Build</span><span>${BUILD}</span></div>
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

function setRow(id, icon, title, sub, opts = {}) {
  const tone = opts.tone ? ` tone-${opts.tone}` : "";
  const tail = opts.value
    ? `<span class="set-val mono">${escapeHtml(opts.value)}</span>`
    : `<span class="chev" aria-hidden="true">›</span>`;
  return `<button class="set-row${tone}" id="${id}">
    <span class="ico" aria-hidden="true">${icon}</span>
    <span class="txt">
      <span class="t">${escapeHtml(title)}</span>
      <span class="s">${escapeHtml(sub)}</span>
    </span>
    ${tail}
  </button>`;
}

function renderSettings() {
  let info = { birth_height: "—", scanned_to: "—", outputs: "—" };
  try {
    info = parseJson(wallet_info(state));
  } catch (_) {}

  const I = {
    key: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><circle cx="8" cy="15" r="4"/><path d="M10.8 12.2 20 3M17 6l2 2M14 9l2 2"/></svg>`,
    eye: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/></svg>`,
    sync: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M21 12a9 9 0 1 1-2.6-6.4"/><path d="M21 3v6h-6"/></svg>`,
    node: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/></svg>`,
    book: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H19v15H6.5A2.5 2.5 0 0 0 4 20.5Z"/><path d="M4 20.5A2.5 2.5 0 0 1 6.5 18H19v3H6.5A2.5 2.5 0 0 1 4 20.5Z"/><path d="M9 7.5h6M9 11h4"/></svg>`,
    trash: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/></svg>`,
  };

  screen(`
    <div class="screen">
      <header class="page-head">
        <p class="kicker">Settings</p>
        <h1>Your wallet</h1>
        <p class="lede">Keys live in this browser. Nothing here is sent anywhere.</p>
      </header>

      <p class="group-label">Keys and backup</p>
      <div class="set-group">
        ${setRow("show-seed", I.key, "Recovery phrase", "24 words · the only way back to these coins")}
        ${setRow("show-view", I.eye, "View key", "Reads amounts and memos · cannot spend")}
      </div>

      <p class="group-label">Contacts</p>
      <div class="set-group">
        ${setRow("open-book", I.book, "Address book", `${loadBook().length} saved · this browser only`)}
      </div>

      <p class="group-label">Sync</p>
      <div class="set-group">
        <div class="set-stats">
          <div><span class="k">Birth</span><span class="v mono">${escapeHtml(String(info.birth_height))}</span></div>
          <div><span class="k">Scanned to</span><span class="v mono">${escapeHtml(String(info.scanned_to))}</span></div>
          <div><span class="k">Unspent</span><span class="v mono">${escapeHtml(String(info.outputs))}</span></div>
        </div>
        ${setRow("rescan", I.sync, "Rescan from birth height", "Forgets what it found and walks the chain again")}
      </div>

      <p class="group-label">Connection</p>
      <div class="set-group">
        <div class="kv"><span class="k">Node</span><span class="v mono">${escapeHtml(nodeUrl() || "seed.nightfallcoin.org")}</span></div>
        <div class="kv"><span class="k">Route</span><span class="v mono">/wallet-api</span></div>
        <div class="kv"><span class="k">Tip</span><span class="v mono">${escapeHtml(String(lastTip || lastBal?.tip || "—"))}</span></div>
        <div class="kv"><span class="k">Protocol</span><span class="v mono">v8 · wire v6</span></div>
        <div class="kv"><span class="k">Build</span><span class="v mono">${BUILD}</span></div>
      </div>

      <details class="note">
        <summary>What this wallet cannot promise</summary>
        <p>${WARN}</p>
      </details>

      <p class="group-label danger">Danger</p>
      <div class="set-group danger">
        ${setRow("wipe", I.trash, "Remove wallet from this browser", "Needs the 24 words to come back", { tone: "danger" })}
      </div>

      <p class="foot-link"><a href="/">nightfallcoin.org</a></p>
    </div>
    ${nav()}
  `);
  bindNav();
  $("#open-book").onclick = () => {
    bookFrom = "settings";
    sheet = "book";
    renderApp();
  };
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

function renderBook() {
  const book = loadBook();
  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Back</button>
      <h1>Address book</h1>
      <p class="hint">Names for addresses you pay often. Stored in this browser
        only — never sent anywhere, and not part of the wallet backup. Clearing
        site data clears this list; the 24 words do not restore it.</p>

      <div class="field">
        <label for="b-label">Name</label>
        <input id="b-label" placeholder="Rent, Anna, exchange…" autocomplete="off">
      </div>
      <div class="field">
        <label for="b-addr">Address</label>
        <input id="b-addr" placeholder="nf1…" spellcheck="false" autocomplete="off" autocapitalize="none">
        <p class="field-err" id="b-err"></p>
      </div>
      <button class="primary" id="b-add">Add to book</button>

      <p class="group-label" style="margin-top:26px">${book.length} saved</p>
      <div class="set-group">
        ${
          book.length
            ? book
                .map(
                  (e, i) => `<div class="book-row">
                    <span class="txt">
                      <span class="t">${escapeHtml(e.label)}</span>
                      <span class="s mono">${escapeHtml(shortAddr(e.addr))}</span>
                    </span>
                    <button type="button" class="rm" data-i="${i}" aria-label="Remove ${escapeHtml(e.label)}">Remove</button>
                  </div>`,
                )
                .join("")
            : `<p class="hint" style="padding:16px;margin:0">Nothing saved yet.</p>`
        }
      </div>
    </div>
    ${nav()}
  `);
  bindNav();
  $("#back").onclick = () => {
    if (bookFrom === "settings") {
      sheet = null;
      tab = "settings";
    } else {
      sheet = "send";
    }
    renderApp();
  };
  $("#b-add").onclick = () => {
    const label = $("#b-label").value.trim();
    const addr = $("#b-addr").value.trim();
    if (!addr.startsWith("nf1") || addr.length < 20) {
      $("#b-err").textContent = "That does not look like an nf1 address.";
      return;
    }
    addToBook(label, addr);
    renderBook();
  };
  document.querySelectorAll(".book-row .rm").forEach((b) => {
    b.onclick = () => {
      const list = loadBook();
      const gone = list[Number(b.dataset.i)];
      if (!gone) return;
      if (!confirm(`Remove "${gone.label}" from the book? The coins are not affected.`)) return;
      list.splice(Number(b.dataset.i), 1);
      saveBook(list);
      renderBook();
    };
  });
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
      <p class="hint">Paper first, a password manager second. Anyone who reads these
        twenty-four words can spend every coin this wallet holds.</p>
      <div id="secret" hidden>
        ${wordGrid(phrase)}
        <p class="hint" style="margin-top:14px">Write them in this order. Order is part of the key.</p>
      </div>
      <button class="primary" id="reveal">Show the 24 words</button>
      <button class="ghost" id="copy" hidden>Copy all 24 words</button>
      <p class="ok" id="copied" hidden></p>
      ${err ? `<p class="warn">${escapeHtml(err)}</p>` : ""}
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
    const ok = await copyText(phrase);
    const n = $("#copied");
    if (n) {
      n.hidden = false;
      n.textContent = ok
        ? "Copied. Clear the clipboard when you have stored them."
        : "Could not copy. Select the words and copy them yourself.";
      n.className = ok ? "ok" : "warn";
    }
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
      <p class="hint">Hand this to an accountant and they see every amount and memo
        you receive — and cannot move a single coin. Spending needs the phrase.</p>
      <div class="secret mono">${escapeHtml(key)}</div>
      <button class="primary" id="copy">Copy view key</button>
      <p class="ok" id="copied" hidden></p>
      ${err ? `<p class="warn">${escapeHtml(err)}</p>` : ""}
    </div>
  `);
  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#copy").onclick = async () => {
    const ok = await copyText(key);
    const n = $("#copied");
    if (n) {
      n.hidden = false;
      n.textContent = ok ? "Copied to the clipboard." : "Could not copy — select the key and copy it yourself.";
      n.className = ok ? "ok" : "warn";
    }
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
  const book = loadBook();
  const availStr = lastBal?.available ?? "0";
  const avail = Number(availStr) || 0;
  const fee = Number(FEE);

  screen(`
    <div class="screen">
      <button class="linkish" id="back">← Wallet</button>
      <h1>Send</h1>
      <p class="hint">Amounts and the recipient are hidden on the chain. The fee
        is burned while blocks still pay a subsidy — no miner receives it.</p>

      <div class="field">
        <label for="to">To address</label>
        <input id="to" placeholder="nf1…" spellcheck="false" autocomplete="off" autocapitalize="none">
        ${
          book.length
            ? `<div class="chips" id="chips">${book
                .map(
                  (e, i) =>
                    `<button type="button" class="chip" data-i="${i}" title="${escapeHtml(e.addr)}">${escapeHtml(e.label)}</button>`,
                )
                .join("")}</div>`
            : ""
        }
        <div class="field-actions">
          <button type="button" class="linkish" id="save-addr" hidden>Save this address</button>
          <button type="button" class="linkish" id="open-book">Address book</button>
        </div>
        <p class="field-note hot">There is no undo and no support desk. An address
          typed wrong is a payment to nobody.</p>
        <p class="field-err" id="err-to"></p>
      </div>

      <div class="field">
        <label for="amt">Amount</label>
        <div class="amount-box">
          <input id="amt" placeholder="0.00" inputmode="decimal" autocomplete="off">
          <span class="unit">NIGHT</span>
          <button type="button" class="max" id="max">MAX</button>
        </div>
        <p class="field-note">Spendable ${fmtAmt(availStr)} NIGHT</p>
        <p class="field-err" id="err-amt"></p>
      </div>

      <div class="field">
        <label for="memo">Memo <span class="dim">— optional, encrypted</span></label>
        <input id="memo" placeholder="Only the recipient can read it" autocomplete="off">
      </div>

      <div class="summary" id="summary">
        <div class="line"><span class="k">They receive</span><span class="v" id="s-amt">—</span></div>
        <div class="line"><span class="k">Fee, burned</span><span class="v">${FEE} NIGHT</span></div>
        <div class="line total"><span class="k">Leaves your wallet</span><span class="v" id="s-total">—</span></div>
        <div class="line" id="s-rest-line"><span class="k">Spendable after</span><span class="v" id="s-rest">${fmtAmt(availStr)}</span></div>
      </div>

      <button class="primary" id="go">Review</button>
      <p id="err" class="warn"></p>
    </div>
  `);

  const $to = $("#to");
  const $amt = $("#amt");
  const trim = (n) => {
    const t = n.toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
    return t || "0";
  };

  function recompute() {
    const v = Number($amt.value.trim());
    const ok = $amt.value.trim() !== "" && Number.isFinite(v) && v > 0;
    $("#s-amt").textContent = ok ? `${fmtAmt(trim(v))} NIGHT` : "—";
    $("#s-total").textContent = ok ? `${fmtAmt(trim(v + fee))} NIGHT` : "—";
    const rest = avail - (ok ? v + fee : 0);
    $("#s-rest").textContent = `${fmtAmt(trim(Math.max(rest, 0)))} NIGHT`;
    // Saying "not enough" here, next to the number, beats saying it after the
    // proofs have already been built.
    $("#s-rest-line").classList.toggle("short", ok && rest < 0);
    $("#err-amt").textContent = ok && rest < 0 ? "More than you can spend, fee included." : "";
  }

  $amt.oninput = recompute;
  $("#max").onclick = () => {
    $amt.value = trim(Math.max(avail - fee, 0));
    recompute();
    $amt.focus();
  };
  function refreshSaveBtn() {
    const v = $to.value.trim();
    const known = v && bookLabel(v);
    const b = $("#save-addr");
    if (b) b.hidden = !(v.startsWith("nf1") && v.length >= 20 && !known);
  }

  $to.oninput = () => {
    $("#err-to").textContent = "";
    refreshSaveBtn();
  };
  refreshSaveBtn();

  document.querySelectorAll("#chips .chip").forEach((c) => {
    c.onclick = () => {
      $to.value = book[Number(c.dataset.i)].addr;
      $("#err-to").textContent = "";
      refreshSaveBtn();
      $amt.focus();
    };
  });

  const saveBtn = $("#save-addr");
  if (saveBtn) {
    saveBtn.onclick = () => {
      const addr = $to.value.trim();
      const label = prompt("Name for this address? It stays in this browser.");
      if (label === null) return;
      addToBook(label, addr);
      renderSend();
    };
  }
  $("#open-book").onclick = () => {
    bookFrom = "send";
    sheet = "book";
    renderApp();
  };

  recompute();

  $("#back").onclick = () => {
    sheet = null;
    renderApp();
  };
  $("#go").onclick = () => {
    const to = $to.value.trim();
    const amt = $amt.value.trim();
    const memo = $("#memo").value;
    $("#err-to").textContent = "";
    $("#err-amt").textContent = "";
    if (!to.startsWith("nf1") || to.length < 20) {
      $("#err-to").textContent = "That does not look like an nf1 address.";
      $to.focus();
      return;
    }
    const v = Number(amt);
    if (!amt || !Number.isFinite(v) || v <= 0) {
      $("#err-amt").textContent = "Enter an amount.";
      $amt.focus();
      return;
    }
    if (v + fee > avail) {
      $("#err-amt").textContent = "More than you can spend, fee included.";
      $amt.focus();
      return;
    }
    if (!confirm(`Send ${fmtAmt(trim(v))} NIGHT to\n${shortAddr(to)}\n\nFee ${FEE} NIGHT, burned.\nLeaves your wallet: ${fmtAmt(trim(v + fee))} NIGHT`)) return;
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
    const built = parseJson(wasmCall(build_send, state, to, amt, memo, tip, Date.now() / 1000));
    lastStatus = "broadcasting…";
    renderHome();
    const res = await rpc("submit_tx", { tx: built.tx });
    state = built.state;
    rememberOutbound(res.txid || built.txid, built.tx);
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
    await flushOutbox();
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
