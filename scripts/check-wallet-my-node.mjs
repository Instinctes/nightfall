#!/usr/bin/env node
/* My Node: the web wallet against a plain node instead of nightfallcoin.org.
 *
 * Written on 30 September 2026 (Roadmap 2027, milestone 4, local only).
 *
 * nightfallcoin.org's Worker answers `wallet_scan` by reading a page's headers,
 * the scan feed, and the headers again. A self-hosted node behind a same-origin
 * reverse proxy has no `wallet_scan`, so `nodeClient` falls back to doing the
 * same three reads in the page (`anchoredScan`). This harness drives the real
 * scan loop through that fallback against a simulated plain node and checks
 * that it is exactly as strict as the Worker: a page that changed between the
 * reads, a gap in the headers, or history the node has pruned is refused.
 *
 *     node scripts/check-wallet-my-node.mjs
 */
import assert from "node:assert/strict";
import { syncWallet, nodeClient, anchoredScan, SCAN_PAGE } from "../website/public/wallet-origin/session.js";

const GENESIS = "06".repeat(32);
const hex = (salt, h) => (h === 0 ? "0".repeat(64) : salt.repeat(2) + String(h).padStart(62, "0"));

/** A node exposing only what `nightfalld --mobile-listen` exposes. */
function plainNode({ tip, salt = "a", prunedBelow = 0 }) {
    const node = {
        tip, salt, prunedBelow, calls: [], limitedFor: 0,
        // Hooks a test can set to misbehave at a precise moment.
        afterFeed: null, gapAt: null,
    };
    node.hash = h => hex(node.salt, h);
    node.rpc = async (method, params = {}) => {
        node.calls.push(method);
        await new Promise(r => setTimeout(r, 0));
        if (node.limitedFor > 0) { node.limitedFor--; throw new Error("rate limited"); }
        switch (method) {
            case "status":
                return { network: "mainnet", genesis: GENESIS, tip_height: node.tip, blocks: node.tip + 1,
                    tip: node.hash(node.tip), loading: false, reorg_in_flight: false, stalled_on_fork: false };
            case "get_headers": {
                const { from, limit } = params;
                if (from < node.prunedBelow) return { first_height: node.prunedBelow, headers: [] };
                const last = Math.min(from + Math.min(limit, 512) - 1, node.tip);
                const headers = [];
                for (let h = from; h <= last; h++) {
                    headers.push({ height: h, hash: node.hash(h), prev_hash: h ? node.hash(h - 1) : "0".repeat(64) });
                }
                if (node.gapAt !== null && headers.length > 2) headers[1].prev_hash = "f".repeat(64);
                return { first_height: node.prunedBelow, headers };
            }
            case "scan_feed": {
                const { from, limit } = params;
                const last = Math.min(from + limit - 1, node.tip);
                const outputs = [];
                for (let h = from; h <= last; h++) outputs.push(`out-${h}`);
                const page = { genesis: GENESIS, from, blocks: last - from + 1, scanned_to: last,
                    tip_height: node.tip, available_from: node.prunedBelow, outputs, spent: [] };
                node.afterFeed?.();
                return page;
            }
            default:
                // What mobile.rs answers for anything outside its allow-list.
                throw new Error(`method '${method}' is not available on the mobile API`);
        }
    };
    return node;
}

function wallet(birth = 0) {
    const live = { birth, scan_from: birth, scanned_to: birth, anchor: "", outputs: [], writes: 0,
        ingestAnchoredPage(outputsJson, _spent, from, scannedTo, fromHash, scannedHash) {
            assert.equal(from, this.scan_from, "page must continue the wallet position");
            if (this.anchor) assert.equal(this.anchor, fromHash, "page must continue the wallet anchor");
            for (const o of JSON.parse(outputsJson)) if (!this.outputs.includes(o)) this.outputs.push(o);
            Object.assign(this, { scanned_to: scannedTo, scan_from: scannedTo, anchor: scannedHash });
        } };
    const session = {
        vault: { info: () => JSON.stringify({ birth_height: live.birth, scan_from: live.scan_from,
            scanned_to: live.scanned_to, scan_anchor: live.anchor, needs_rebuild: false }) },
        async edit(change) {
            const copy = Object.assign(Object.create(Object.getPrototypeOf(live)), live, { outputs: [...live.outputs] });
            change(copy);
            Object.assign(live, copy, { writes: live.writes + 1 });
        },
    };
    return { live, session };
}

const noWait = () => Promise.resolve();
let failures = 0;
const check = async (name, body) => {
    try { await body(); console.log(`  ok   ${name}`); }
    catch (e) { failures++; console.error(`  FAIL ${name}\n       ${e.message}`); }
};

console.log("web wallet against a plain node");

await check("a full sync against a plain node reaches the tip with every block", async () => {
    const node = plainNode({ tip: 3_000 });
    const rpc = nodeClient(node.rpc, { wait: noWait });
    const { live, session } = wallet();
    await syncWallet({ session, rpc, genesis: GENESIS, assertCurrent: () => {} });
    assert.equal(live.scanned_to, 3_000);
    assert.equal(live.outputs.length, 3_001);
    assert.ok(rpc.scansLocally(), "the client should have switched to scanning in the page");
    // One refused wallet_scan, then only the three plain reads per page.
    assert.equal(node.calls.filter(c => c === "wallet_scan").length, 1);
});

await check("an endpoint that has wallet_scan is used as is", async () => {
    const node = plainNode({ tip: 100 });
    const worker = async (method, params) =>
        method === "wallet_scan" ? anchoredScan(node.rpc, params) : node.rpc(method, params);
    const seen = [];
    const rpc = nodeClient(async (m, p) => { seen.push(m); return worker(m, p); }, { wait: noWait });
    const { live, session } = wallet();
    await syncWallet({ session, rpc, genesis: GENESIS, assertCurrent: () => {} });
    assert.equal(live.scanned_to, 100);
    assert.ok(!rpc.scansLocally());
    assert.ok(!seen.includes("get_headers"), "the page must not do the Worker's job twice");
});

await check("a reorg between the reads refuses the page and saves nothing", async () => {
    const node = plainNode({ tip: 50 });
    node.afterFeed = () => { node.salt = "b"; };
    const rpc = nodeClient(node.rpc, { wait: noWait });
    const { live, session } = wallet();
    await assert.rejects(syncWallet({ session, rpc, genesis: GENESIS, assertCurrent: () => {} }), /chain changed/);
    assert.equal(live.writes, 0);
    assert.equal(live.outputs.length, 0);
});

await check("a gap in the headers is refused", async () => {
    const node = plainNode({ tip: 50 });
    node.gapAt = 1;
    await assert.rejects(anchoredScan(node.rpc, { from: 0, limit: 10 }), /discontinuous/);
});

await check("history a pruned node no longer has is refused, not skipped", async () => {
    const node = plainNode({ tip: 5_000, prunedBelow: 4_000 });
    await assert.rejects(anchoredScan(node.rpc, { from: 10, limit: 10 }), /history is unavailable/);
});

await check("out-of-range requests never reach the node", async () => {
    const node = plainNode({ tip: 50 });
    for (const bad of [{ from: -1, limit: 1 }, { from: 0, limit: 0 }, { from: 0, limit: SCAN_PAGE + 1 }, { from: 1.5, limit: 2 }]) {
        await assert.rejects(anchoredScan(node.rpc, bad), /Invalid scan range/);
    }
    assert.equal(node.calls.length, 0);
});

await check("a rate-limited node is waited out with a doubling delay", async () => {
    const node = plainNode({ tip: 20 });
    node.limitedFor = 2;
    const waits = [];
    const rpc = nodeClient(node.rpc, { wait: ms => { waits.push(ms); return Promise.resolve(); } });
    const status = await rpc("status");
    assert.equal(status.tip_height, 20);
    assert.deepEqual(waits, [1000, 2000]);
});

await check("a node that stays rate-limited is reported, not retried forever", async () => {
    const node = plainNode({ tip: 20 });
    node.limitedFor = 100;
    const rpc = nodeClient(node.rpc, { retries: 3, wait: noWait });
    await assert.rejects(rpc("status"), /rate limited/);
    assert.equal(node.calls.length, 4);
});

await check("other wallet_scan failures do not silently switch modes", async () => {
    const rpc = nodeClient(async () => { throw new Error("Node could not supply the scan page."); }, { wait: noWait });
    await assert.rejects(rpc("wallet_scan", { from: 0, limit: 1 }), /could not supply/);
    assert.ok(!rpc.scansLocally());
});

console.log(failures ? `\n${failures} failing` : "\nplain-node wallet ok");
process.exit(failures ? 1 : 0);
