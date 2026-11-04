#!/usr/bin/env node
/* My Node, end to end against a REAL local devnet node (no mainnet, no network).
 *
 * Start a devnet node with its light API on loopback first, e.g.
 *
 *   nightfalld --network devnet --datadir /tmp/nf-mynode-e2e/data init
 *   nightfalld --network devnet --datadir /tmp/nf-mynode-e2e/data mine --blocks 40 --miner-seed /tmp/nf-mynode-e2e/miner.seed
 *   nightfalld --network devnet --datadir /tmp/nf-mynode-e2e/data run \
 *       --listen 127.0.0.1:27891 --rpc-listen 127.0.0.1:27881 --mobile-listen 127.0.0.1:27888
 *
 * then:   node scripts/e2e-my-node-devnet.mjs http://127.0.0.1:27888/ 7
 *
 * What it proves, against the real node code rather than a simulation:
 *  1. `wallet_scan` is refused by a plain node and `nodeClient` falls back to
 *     bracketed scanning in the page, covering every block from 0 to the tip
 *     in continuous, hash-linked pages.
 *  2. The node's rate limit is charged per forwarded client when the request
 *     comes from loopback (what a same-host reverse proxy looks like), so one
 *     busy visitor cannot lock out the others.
 */
import assert from "node:assert/strict";
import { nodeClient, SCAN_PAGE } from "../website/public/wallet-origin/session.js";

const url = process.argv[2] || "http://127.0.0.1:27888/";
// Page size; small values exercise many page boundaries on a short devnet chain.
const PAGE = Math.min(SCAN_PAGE, Number(process.argv[3]) || SCAN_PAGE);

async function post(method, params = {}, headers = {}) {
    const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json", ...headers },
        body: JSON.stringify({ method, params, id: 1 }) });
    const body = await r.json().catch(() => ({}));
    return { status: r.status, body };
}
const raw = async (method, params) => {
    const { status, body } = await post(method, params);
    if (status !== 200 || body.error) throw new Error(typeof body.error === "string" ? body.error : body.error?.message || `HTTP ${status}`);
    return body.result;
};

const status = await raw("status");
console.log(`node: ${status.network} tip ${status.tip_height}, genesis ${status.genesis.slice(0, 12)}…`);
assert.notEqual(status.network, "mainnet", "this exercise runs against devnet only");

// 1. Full scan through the fallback.
const rpc = nodeClient(raw, { baseDelay: 200 });
let from = 0, pages = 0, anchor = null, outputs = 0;
const t0 = Date.now();
while (true) {
    const limit = Math.min(PAGE, status.tip_height - from + 1);
    const page = await rpc("wallet_scan", { from, limit });
    if (anchor) assert.equal(page.from_hash, anchor, `page at ${from} does not continue the previous one`);
    assert.equal(page.from, from);
    assert.equal(page.scanned_to, from + limit - 1);
    anchor = page.scanned_hash; outputs += page.outputs.length; pages++;
    if (page.scanned_to >= status.tip_height) break;
    from = page.scanned_to; // one-block overlap, exactly as the wallet does
}
assert.ok(rpc.scansLocally(), "a plain node should have pushed the client to scan in the page");
assert.equal(anchor, status.tip, "the last page must end on the node's tip hash");
console.log(`  ok   scanned 0..${status.tip_height} in ${pages} bracketed pages (${outputs} outputs seen, ${Date.now() - t0} ms)`);

// 2. Per-client budgets behind a loopback proxy. Wait out the window the scan used.
console.log("  ..   waiting 61 s for the rate window to reset");
await new Promise(r => setTimeout(r, 61_000));
let firstRefused = null;
for (let i = 1; i <= 125; i++) {
    const { status: code } = await post("status", {}, { "X-Forwarded-For": "198.51.100.1" });
    if (code === 429) { firstRefused = i; break; }
}
assert.equal(firstRefused, 121, `visitor A should be refused on request 121, was ${firstRefused}`);
const other = await post("status", {}, { "X-Forwarded-For": "198.51.100.2" });
assert.equal(other.status, 200, "visitor B must still be served while A is limited");
const spoofChain = await post("status", {}, { "X-Forwarded-For": "198.51.100.2, 198.51.100.1" });
assert.equal(spoofChain.status, 429, "the proxy-appended (last) entry decides, not what a visitor prepends");
console.log("  ok   visitor A limited at 121, visitor B unaffected, prepended addresses ignored");
console.log("\nMy Node end-to-end ok");
