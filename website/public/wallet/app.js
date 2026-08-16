import init, {
  create_wallet,
  restore_wallet,
  wallet_address,
  wallet_scan_from,
  ingest_page,
  wallet_balance,
  wallet_history,
  build_send,
} from "./pkg/nightfall_web.js";

const STORE = "nf-web-wallet-v1";
const $ = (s) => document.querySelector(s);
const app = $("#app");

const WARN =
  "This phone or browser trusts a node for what it shows. A hostile node can hide a payment or invent one on the screen. It cannot spend — the seed never leaves this device. Anyone who can run script on this page can read a saved wallet. The 24 words are the real backup.";

let wasmReady = init();
let state = null;
let phrasePending = null;

function save() {
  if (state) localStorage.setItem(STORE, state);
}
function loadSaved() {
  return localStorage.getItem(STORE);
}
function forget() {
  localStorage.removeItem(STORE);
  state = null;
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

function screen(html) {
  app.innerHTML = html;
}

function onboard() {
  screen(`
    <p class="brand">NIGHT</p>
    <h1>Wallet in the browser</h1>
    <p class="lede">Receive and send without installing an app. Add this page to your home screen.</p>
    <div class="card">
      <p class="hint">${WARN}</p>
    </div>
    <button id="create">Create a wallet</button>
    <button class="ghost" id="restore-toggle">I have 24 words</button>
    <div id="restore" hidden>
      <textarea id="words" placeholder="twenty four words…"></textarea>
      <input id="birth" placeholder="Birth height (0 if unsure)" value="0">
      <p class="hint">A number that is too high silently misses coins. Too low only costs time.</p>
      <button id="do-restore">Restore</button>
    </div>
    <p id="err" class="warn"></p>
    <p class="hint"><a href="/">Back to nightfallcoin.org</a></p>
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
    <h1>Write these 24 words down</h1>
    <p class="hint">On paper. Not a screenshot, not the cloud. Nobody can reset this.</p>
    <div class="card mono">${phrase}</div>
    <p class="hint">Type word 4 and word 18 to continue.</p>
    <input id="w4" placeholder="Word 4" autocomplete="off">
    <input id="w18" placeholder="Word 18" autocomplete="off">
    <button id="done" disabled>I have the words</button>
    <p id="err" class="warn"></p>
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
    home();
  };
}

async function home() {
  let address = "";
  try {
    address = wallet_address(state);
  } catch (e) {
    screen(`<p class="warn">${e}</p><button id="reset">Start over</button>`);
    $("#reset").onclick = () => {
      forget();
      onboard();
    };
    return;
  }
  screen(`
    <p class="brand">NIGHT</p>
    <p class="total" id="total">—</p>
    <p class="dim" id="split">spendable — · unlocking —</p>
    <p class="hint" id="status">tap Sync</p>
    <div class="row">
      <button id="sync">Sync</button>
      <button class="ghost" id="copy">Copy address</button>
    </div>
    <div class="card">
      <p class="hint">Your address</p>
      <p class="mono" id="addr">${address}</p>
    </div>
    <h2>Send</h2>
    <p class="hint">Wrong address = gone forever. Fee 0.001 NIGHT, burned while blocks still pay a subsidy.</p>
    <input id="to" placeholder="nf1…" spellcheck="false" autocomplete="off">
    <input id="amt" placeholder="Amount" inputmode="decimal">
    <input id="memo" placeholder="Memo (optional)">
    <button id="send">Send</button>
    <h2>Activity</h2>
    <div id="hist"></div>
    <p class="hint" style="margin-top:2rem">${WARN}</p>
    <p class="hint"><button class="ghost" id="wipe">Remove wallet from this browser</button></p>
  `);
  $("#sync").onclick = sync;
  $("#copy").onclick = async () => {
    await navigator.clipboard.writeText(address);
    $("#status").textContent = "address copied";
  };
  $("#send").onclick = send;
  $("#wipe").onclick = () => {
    if (confirm("This removes the wallet from this browser only. You need the 24 words to get it back.")) {
      forget();
      onboard();
    }
  };
  await sync();
}

async function sync() {
  const status = $("#status");
  if (!status) return;
  status.textContent = "syncing…";
  try {
    const tipPage = await rpc("status", {});
    const tip = tipPage.tip_height ?? Math.max(0, (tipPage.blocks || 1) - 1);
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
    const bal = parseJson(wallet_balance(state, tip));
    $("#total").textContent = bal.total + " NIGHT";
    $("#split").textContent = `spendable ${bal.available}  ·  unlocking ${bal.immature}`;
    status.textContent = found ? `found ${found} new output(s)` : "up to date";
    const rows = parseJson(wallet_history(state));
    $("#hist").innerHTML = rows.length
      ? rows
          .map((e) => {
            const extra = e.pending ? "pending" : e.height != null ? `#${e.height}` : "";
            return `<div class="hist"><strong>${e.direction}</strong> ${e.amount} <span class="dim">${extra}</span>${
              e.memo ? `<div class="dim">${escapeHtml(e.memo)}</div>` : ""
            }</div>`;
          })
          .join("")
      : `<p class="dim">No movements yet.</p>`;
  } catch (e) {
    status.innerHTML = `<span class="warn">${escapeHtml(e.message || String(e))}</span>`;
  }
}

async function send() {
  const status = $("#status");
  status.textContent = "sending…";
  try {
    const tipPage = await rpc("status", {});
    const tip = tipPage.tip_height ?? 0;
    const built = parseJson(
      build_send(state, $("#to").value.trim(), $("#amt").value.trim(), $("#memo").value, tip),
    );
    const res = await rpc("submit_tx", { tx: built.tx });
    state = built.state;
    save();
    status.textContent = "sent " + (res.txid || built.txid);
    $("#to").value = "";
    $("#amt").value = "";
    $("#memo").value = "";
    await sync();
  } catch (e) {
    status.innerHTML = `<span class="warn">${escapeHtml(e.message || String(e))}</span>`;
  }
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

async function onCreate() {
  try {
    await wasmReady;
    const tip = await rpc("status", {}).then((s) => s.tip_height ?? 0).catch(() => 0);
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
    await home();
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
      await home();
    } else {
      onboard();
    }
  } catch (e) {
    screen(`<p class="warn">Could not start the wallet: ${escapeHtml(e.message || e)}</p>`);
  }
}

boot();
