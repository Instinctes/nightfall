#!/usr/bin/env node
/* Checkout prototype, end to end on an isolated devnet. Loopback only.
 *
 * Roadmap 2027, milestone 6. Written 30 September 2026.
 *
 * Real processes: a merchant node, a miner node, a merchant wallet exporting a view-only credential,
 * a customer wallet, and later a rival miner. The question under test is the
 * one a shop actually has: *may I hand over the goods?*
 *
 *   1. The merchant writes an invoice; the till prints a payment request.
 *      The customer pays it with the request's memo. The till initially reports "not paid". On a stable real chain we verify
 *      "confirming" one below the selected threshold and "confirmed" exactly
 *      at it, even if fast mining skips a sampled intermediate state.
 *   2. A second invoice is paid and mined, and while it is still confirming a
 *      heavier rival branch without that payment replaces the merchant's
 *      chain. The till must drop the payment — "not paid" — and must never
 *      have said "confirmed" for it. The first invoice, below the fork, stays
 *      confirmed.
 *
 * A payment request is not a payment and a block one deep is not final; this
 * run is the evidence for both. Devnet only: the wallet refuses the checkout
 * commands on mainnet.
 *
 *     cargo build -p nightfall-node --bin nightfalld -p nightfall-wallet
 *     node scripts/lab/checkout-devnet.mjs [--out output/lab]
 */
import { createCheckoutServer, cliBackend } from "../checkout/devnet-service.mjs";
import { randomBytes } from "node:crypto";
import { once } from "node:events";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, cpSync, existsSync, readdirSync, openSync } from "node:fs";
import { tmpdir, platform, arch } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(new URL("../..", import.meta.url).pathname);
const binDir=resolve(process.env.NIGHTFALL_LAB_BIN_DIR || join(root,"target/debug"));
const nodeBin = join(binDir, "nightfalld");
const walletBin = join(binDir, "nightfall-wallet");
const tillBin = join(binDir, "nightfall-till");
const outArg = process.argv.indexOf("--out");
const outBase = resolve(root, outArg > 0 ? process.argv[outArg + 1] : "output/lab");
for (const b of [nodeBin, walletBin, tillBin]) if (!existsSync(b)) { console.error(`missing ${b}; build it first`); process.exit(2); }

const work = mkdtempSync(join(tmpdir(), "nightfall-checkout-"));
const BASE = 33000 + Math.floor(Math.random() * 300) * 10;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const running = new Set();
const log = [];
const note = (msg, data = {}) => { log.push({ t: new Date().toISOString(), msg, ...data }); console.log(`  ·    ${msg}${Object.keys(data).length ? " " + JSON.stringify(data) : ""}`); };

function mkNode(name, i) {
    return { name, datadir: join(work, name), p2p: BASE + i * 3, rpc: BASE + i * 3 + 1, light: BASE + i * 3 + 2, proc: null };
}
const M = mkNode("merchant-node", 0), P = mkNode("miner-node", 1), R = mkNode("rival-node", 2);
const watchDir = join(work,"view-only-till"), viewFile=join(work,"view-key.txt");
const watch = (...args) => execFileSync(tillBin,["--network","devnet","--datadir",watchDir,"--view-key-file",viewFile,"--rpc",`127.0.0.1:${M.rpc}`,...args],{encoding:"utf8",timeout:120000});
const customerDir = join(work, "customer"), merchantDir = join(work, "merchant");

const nd = (n, ...a) => execFileSync(nodeBin, ["--network", "devnet", "--datadir", n.datadir, ...a], { encoding: "utf8", timeout: 900_000 });
const wallet = (dir, rpcNode, ...a) => execFileSync(walletBin, ["--network", "devnet", "--datadir", dir, "--rpc", `127.0.0.1:${rpcNode.rpc}`, ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 300_000 });

async function start(n, { connect = [], mine = null } = {}) {
    const args = ["--network", "devnet", "--datadir", n.datadir, "run", "--listen", `127.0.0.1:${n.p2p}`,
        "--rpc-listen", `127.0.0.1:${n.rpc}`, "--mobile-listen", `127.0.0.1:${n.light}`, "--proxy", "off",
        ...connect.flatMap(c => ["--connect", `127.0.0.1:${c.p2p}`]),
        ...(mine ? ["--mine", "--miner-seed", mine] : [])];
    const fd = openSync(join(work, `${n.name}.log`), "a");
    n.proc = spawn(nodeBin, args, { stdio: ["ignore", fd, fd], env: { ...process.env, NIGHTFALL_PEERS_URL: "" } });
    running.add(n);
    for (let i = 0; i < 240; i++) {
        try { const s = await status(n); if (!s.loading) return s; } catch { /* starting */ }
        await sleep(250);
    }
    throw new Error(`${n.name} did not start`);
}
async function stop(n) {
    if (!n.proc) return;
    const p = n.proc; n.proc = null; running.delete(n);
    const gone = new Promise(r => p.once("exit", r));
    p.kill("SIGTERM");
    await Promise.race([gone, sleep(10_000)]);
    if (p.exitCode === null && p.signalCode === null) { p.kill("SIGKILL"); await gone; }
}
async function status(n) {
    const r = await fetch(`http://127.0.0.1:${n.light}/status`, { signal: AbortSignal.timeout(3000) });
    return (await r.json()).result;
}
function copyChain(from, to) {
    mkdirSync(to.datadir, { recursive: true });
    for (const f of readdirSync(from.datadir)) if (f !== ".nightfall-lock") cpSync(join(from.datadir, f), join(to.datadir, f), { recursive: true });
}
function till(need) {
    const out = watch("till", "--confirmations", String(need), "--json");
    return Object.fromEntries(out.trim().split("\n").filter(Boolean).map(l => { const j = JSON.parse(l); return [j.reference, j]; }));
}
const verdicts = [];
const verdict = (name, ok, evidence) => { verdicts.push({ name, verdict: ok ? "pass" : "fail", evidence }); console.log(`  ${ok ? "ok  " : "FAIL"} ${name}`); };

let exitCode = 0;
let api;
const apiToken = randomBytes(32).toString("hex");
let apiURL;
const apiHeaders = { authorization: `Bearer ${apiToken}`, "content-type": "application/json" };
try {
    console.log(`Checkout prototype — devnet, loopback ports ${BASE}+, work dir ${work}`);
    // Wallets.
    wallet(customerDir, M, "init");
    wallet(merchantDir, M, "init");
    mkdirSync(watchDir);
    writeFileSync(viewFile,wallet(merchantDir,M,"export-view-key").trim(),{mode:0o600});
    writeFileSync(join(watchDir, ".nightfall-checkout-devnet"), "devnet\n");
    api = createCheckoutServer({ token: apiToken, backend: cliBackend({binary: tillBin, datadir: watchDir, viewKeyFile: viewFile, rpc: `127.0.0.1:${M.rpc}`, confirmations: 6}) });
    api.listen(0, "127.0.0.1"); await once(api, "listening");
    apiURL = `http://127.0.0.1:${api.address().port}`;
    const customerSeed = join(customerDir, "wallet.seed");
    // Base chain: 15 blocks to the customer (devnet coinbase maturity is 10).
    nd(P, "init");
    nd(P, "mine", "--blocks", "15", "--miner-seed", customerSeed);
    copyChain(P, M);
    note("base chain mined to the customer", { height: (await (async () => { await start(M); const s = await status(M); await stop(M); return s.tip_height; })()) });

    // ---- 1. Pay an invoice and watch it meet the confirmation threshold.
    await start(M);
    await start(P, { connect: [M], mine: customerSeed });
    const request = watch("invoice", "--reference", "A-1", "--darks", "300000000", "--description", "lab coffee").trim();
    note("invoice A-1 written", { request });
    let t = till(6);
    const seen1 = [t["A-1"].settlement];
    wallet(customerDir, P, "sync");
    const sent = wallet(customerDir, P, "send", "--to", request.match(/nightfall:([^?]+)/)[1], "--amount", "300000000", "--darks", "--memo", "A-1");
    note("customer paid A-1 with the request's memo", { txid: (sent.match(/txid\.+ (\w+)/) || [])[1] });
    let finalAt = null;
    for (let i = 0; i < 240 && !finalAt; i++) {
        t = till(6);
        const s = t["A-1"].settlement;
        if (seen1[seen1.length - 1] !== s) seen1.push(s);
        if (s === "confirmed") finalAt = t["A-1"].tip_height;
        await sleep(500);
    }
    // Stop production and verify the exact threshold on one stable real chain.
    // A fast release miner may legitimately cross six blocks between two scans;
    // skipping a sampled intermediate state is not an early confirmation.
    await stop(P);
    let stableTip = -1;
    for (let i = 0; i < 40; i++) {
        const h = (await status(M)).tip_height;
        if (h === stableTip) break;
        stableTip = h;
        await sleep(250);
    }
    const deep = till(10000)["A-1"];
    const depth = Number(/^confirming:(\d+)\/10000$/.exec(deep.settlement)?.[1]);
    const below = Number.isInteger(depth) && depth > 0 && depth < 10000 ? till(depth + 1)["A-1"] : null;
    const at = below ? till(depth)["A-1"] : null;
    verdict("unpaid invoice is confirming below its exact threshold and confirmed at it",
        finalAt !== null && seen1[0] === "not_paid" && below?.settlement === `confirming:${depth}/${depth + 1}` && at?.settlement === "confirmed" && below.tip_height === at.tip_height,
        { sampled_sequence: seen1, stable_tip: at?.tip_height, confirmations: depth, below: below?.settlement, at: at?.settlement, received_darks: deep.received_darks });

    const created = await fetch(apiURL + "/invoices", {method: "POST", headers: apiHeaders, body: JSON.stringify({reference: "C-3", amount_darks: "123000000", expires_in: 60})});
    const createdBody = await created.json();
    const observed = await fetch(apiURL + "/invoices/A-1", {headers: apiHeaders});
    const observedBody = await observed.json();
    verdict("authenticated HTTP API creates a durable Devnet request and reads confirmed status", created.status === 201 && createdBody.request.includes("network=devnet") && observedBody.settlement === "confirmed" && observedBody.reorg_risk === true, {created: created.status, settlement: observedBody.settlement});
    const pendingAPI = await (await fetch(apiURL + "/invoices/C-3", {headers: apiHeaders})).json();
    verdict("API invoice creation is not a payment", pendingAPI.settlement === "not_paid", {settlement: pendingAPI.settlement});
    const duplicate=await fetch(apiURL+"/invoices",{method:"POST",headers:apiHeaders,body:JSON.stringify({reference:"C-3",amount_darks:"999000000"})});
    const preserved=await (await fetch(apiURL+"/invoices/C-3",{headers:apiHeaders})).json();
    verdict("duplicate invoice cannot overwrite its original amount",duplicate.status===409 && preserved.asked_darks===123000000,{status:duplicate.status,asked:preserved.asked_darks});
    verdict("till data contains invoices only, no seed/spending wallet",readdirSync(watchDir).every(n=>["invoices",".nightfall-checkout-devnet"].includes(n)),{files:readdirSync(watchDir)});
    let tillRefused=false;try{execFileSync(tillBin,["--network","mainnet","--datadir",join(work,"forbidden-mainnet"),"--view-key-file",viewFile,"--rpc",`127.0.0.1:${M.rpc}`,"address"],{stdio:"pipe"});}catch{tillRefused=true;}
    verdict("view-only till refuses mainnet before opening data",tillRefused && !existsSync(join(work,"forbidden-mainnet")),{});

    // ---- 2. Pay a second invoice, then lose it to a heavier rival branch.
    await stop(P); await stop(M);
    copyChain(M, R);                      // the rival forks here, before B-2 exists
    const forkHeight = (await (async () => { await start(R); const s = await status(R); await stop(R); return s.tip_height; })());
    await start(M);
    await start(P, { connect: [M], mine: customerSeed });
    const request2 = watch("invoice", "--reference", "B-2", "--darks", "200000000").trim();
    wallet(customerDir, P, "sync");
    wallet(customerDir, P, "send", "--to", request2.match(/nightfall:([^?]+)/)[1], "--amount", "200000000", "--darks", "--memo", "B-2");
    note("customer paid B-2", { fork_height: forkHeight });
    const seen2 = [];
    let confirming = null;
    for (let i = 0; i < 240 && !confirming; i++) {
        t = till(1000); // a deliberately deep policy: B-2 must still be confirming when the rival arrives
        const s = t["B-2"].settlement;
        if (seen2[seen2.length - 1] !== s) seen2.push(s);
        if (s.startsWith("confirming")) confirming = s;
        await sleep(300);
    }
    if (!confirming) throw new Error(`B-2 never reached a block: ${JSON.stringify(seen2)}`);
    await stop(P);
    const mHeight = (await status(M)).tip_height;
    await stop(M);
    const needed = mHeight - forkHeight + 4;
    note("B-2 is in the merchant's chain; building a heavier rival branch without it", { merchant_height: mHeight, rival_blocks: needed, b2: confirming });
    nd(R, "mine", "--blocks", String(needed), "--miner-seed", join(work, "rival.seed"));
    const rs = await start(R);
    await start(M, { connect: [R] });
    let converged = null;
    for (let i = 0; i < 240 && !converged; i++) {
        const sm = await status(M);
        if (sm.tip === rs.tip) converged = sm;
        await sleep(500);
    }
    if (!converged) throw new Error("merchant node did not adopt the heavier rival branch");
    note("merchant node reorganised onto the rival branch", { tip_height: converged.tip_height });
    t = till(6);
    verdict("a confirming payment removed by a reorg is reported as not paid", t["B-2"].settlement === "not_paid" && !seen2.includes("confirmed"),
        { before: seen2, after: t["B-2"].settlement, received_after: t["B-2"].received_darks });
    verdict("an invoice settled below the fork stays confirmed", t["A-1"].settlement === "confirmed", { a1: t["A-1"].settlement });

    const reorgAPI = await (await fetch(apiURL + "/invoices/B-2", {headers: apiHeaders})).json();
    verdict("HTTP API reports fresh unpaid status after reorg", reorgAPI.settlement === "not_paid", {settlement: reorgAPI.settlement});

    // ---- 3. Mainnet stays off.
    let refused = false;
    try { execFileSync(walletBin, ["--network", "mainnet", "--datadir", join(work, "never"), "till"], { stdio: "pipe" }); }
    catch (e) { refused = /not enabled on mainnet/.test(String(e.stderr)); }
    verdict("the checkout commands refuse mainnet", refused && !existsSync(join(work, "never")), {});
} catch (e) {
    verdicts.push({ name: "run", verdict: "fail", error: e.message });
    console.log(`  FAIL ${e.message}`);
} finally {
    if (api) { api.closeAllConnections(); await new Promise(resolve => api.close(resolve)); }
    for (const n of [...running]) await stop(n);
}

const failed = verdicts.filter(v => v.verdict !== "pass").length;
const commit = (() => { try { return execFileSync("git", ["-C", root, "rev-parse", "--short", "HEAD"], { encoding: "utf8" }).trim(); } catch { return "unknown"; } })();
const out = join(outBase, `checkout-${new Date().toISOString().replace(/[:.]/g, "-")}`);
mkdirSync(out, { recursive: true });
const report = { kind: "nightfall-checkout-devnet", when: new Date().toISOString(), commit, platform: `${platform()}/${arch()}`,
    scope: "devnet, loopback, temp datadirs, SOCKS off; no mainnet", verdicts, log,
    not_covered: ["public merchant deployment", "mainnet", "payment requests from real third-party wallets"] };
writeFileSync(join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
writeFileSync(join(out, "report.md"), [`# Checkout devnet report — ${report.when}`, "", `Commit \`${commit}\` · ${report.platform} · ${report.scope}`, "",
    "| Check | Verdict |", "|---|---|", ...verdicts.map(v => `| ${v.name} | ${v.verdict} |`), "", "## Evidence", "",
    ...verdicts.map(v => `- **${v.name}**: \`${JSON.stringify(v.evidence ?? { error: v.error })}\``), "",
    "## Not covered", "", ...report.not_covered.map(x => `- ${x}`), ""].join("\n"));
console.log(`\n${verdicts.length - failed}/${verdicts.length} checks passed · report: ${out}`);
if (!failed) rmSync(work, { recursive: true, force: true }); else { console.log(`logs kept in ${work}`); exitCode = 1; }
process.exit(exitCode || (failed ? 1 : 0));
