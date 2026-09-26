#!/usr/bin/env node
/* The browser scan loop, driven against a simulated node.
 *
 * Written on 26 September 2026, when the loop was changed to ask for larger
 * pages, keep one page in flight while the previous one is checked, and write
 * to storage once per batch instead of once per page. Every one of those is a
 * chance to lose a page, apply one twice, or stitch two together across a
 * reorg — and none of it shows up in a type checker.
 *
 * The wallet itself is the thing that refuses a page which does not continue
 * its own position and anchor, so the fake vault here enforces exactly that
 * rule and nothing softer. If this harness passes while the real rule is
 * stricter, the harness is wrong; keep them in step.
 *
 *     node scripts/check-wallet-sync.mjs
 */
import assert from "node:assert/strict";
import { syncWallet, SCAN_PAGE, SCAN_BATCH, SCAN_AHEAD } from "../website/public/wallet-origin/session.js";

const GENESIS = "06".repeat(32);
const hashOf = height => String(height).padStart(64, "0");

/** A chain of `length` blocks, each carrying one output at its own height. */
function chain(length, salt = "a") {
    return {
        length,
        hash: h => (h === 0 ? hashOf(0) : salt.repeat(2) + String(h).padStart(62, "0")),
    };
}

/** The wallet's real acceptance rule, and a count of how often it was saved. */
function fakeVault(birth) {
    return {
        birth,
        scan_from: birth,
        scanned_to: birth,
        anchor: "",
        outputs: [],
        writes: 0,
        // The signature the real WASM vault exposes, so the loop is exercised
        // exactly as it calls it.
        ingestAnchoredPage(outputsJson, spentJson, from, scannedTo, fromHash, scannedHash) {
            assert.equal(from, this.scan_from, "page must continue the wallet position");
            if (this.anchor) {
                assert.equal(this.anchor, fromHash, "page must continue the wallet anchor");
            }
            assert.ok(scannedTo - from < 1024, "page exceeds the 1024-block ceiling");
            // Pages overlap by one block, so the boundary block arrives twice.
            // The real wallet ignores an output it already holds; anything
            // beyond that overlap being repeated is a genuine fault.
            for (const output of JSON.parse(outputsJson)) {
                const height = Number(output.slice(4));
                assert.ok(height >= from && height <= scannedTo, `output ${output} outside the page`);
                if (this.outputs.includes(output)) {
                    assert.equal(height, from, `output ${output} ingested twice outside the overlap`);
                    continue;
                }
                this.outputs.push(output);
            }
            JSON.parse(spentJson);
            this.scanned_to = scannedTo;
            this.scan_from = scannedTo;
            this.anchor = scannedHash;
        },
    };
}

function harness({ tip, birth = 0, chainSalt = "a" }) {
    const book = chain(tip + 1, chainSalt);
    const live = fakeVault(birth);
    let requests = 0;
    let inFlight = 0;
    let concurrent = 0;

    const session = {
        vault: {},
        async edit(change) {
            // A real edit forks, applies, snapshots, writes, then adopts. The
            // important part to model is that a failure must leave `live`
            // untouched, so apply to a copy and swap on success.
            const copy = Object.assign(Object.create(Object.getPrototypeOf(live)), live, {
                outputs: [...live.outputs],
            });
            change(copy);
            Object.assign(live, copy, { writes: live.writes + 1 });
        },
    };

    const rpc = async (method, params) => {
        if (method === "status") {
            return { network: "mainnet", genesis: GENESIS, tip_height: tip, blocks: tip + 1,
                tip: book.hash(tip), loading: false, reorg_in_flight: false, stalled_on_fork: false };
        }
        assert.equal(method, "wallet_scan");
        requests++;
        inFlight++;
        concurrent = Math.max(concurrent, inFlight);
        await new Promise(resolve => setTimeout(resolve, 1));
        inFlight--;
        const { from, limit } = params;
        assert.ok(limit >= 1 && limit <= SCAN_PAGE, `limit ${limit} outside 1..${SCAN_PAGE}`);
        const last = Math.min(from + limit - 1, tip);
        const outputs = [];
        for (let h = from; h <= last; h++) outputs.push(`out-${h}`);
        return { genesis: GENESIS, from, blocks: last - from + 1, scanned_to: last,
            tip_height: tip, available_from: 0, outputs, spent: [],
            from_hash: book.hash(from), scanned_hash: book.hash(last) };
    };

    return { live, rpc, session, inFlight: () => inFlight, stats: () => ({ requests, concurrent, writes: live.writes }) };
}

// The harness wires info() to the live vault for every case.
const withInfo = options => {
    const h = harness(options);
    h.session.vault.info = () => JSON.stringify({
        birth_height: h.live.birth, scan_from: h.live.scan_from, scanned_to: h.live.scanned_to,
        scan_anchor: h.live.anchor, needs_rebuild: false,
    });
    return h;
};

async function sync(options) {
    const h = withInfo(options);
    const status = await syncWallet({
        session: h.session, rpc: h.rpc, genesis: GENESIS, assertCurrent: () => {},
    });
    return { status, ...h.stats(), live: h.live };
}

let failures = 0;
const check = async (name, body) => {
    try { await body(); console.log(`  ok   ${name}`); }
    catch (error) { failures++; console.error(`  FAIL ${name}\n       ${error.message}`); }
};

console.log("browser scan loop");

await check("every block is scanned exactly once, in order", async () => {
    const tip = 5_000;
    const { live } = await sync({ tip });
    assert.equal(live.scanned_to, tip);
    assert.equal(live.outputs.length, tip + 1);
    assert.equal(live.outputs[0], "out-0");
    assert.equal(live.outputs[tip], `out-${tip}`);
});

await check("a wallet born at the tip finishes in one request", async () => {
    const tip = 236_220;
    const { requests, live } = await sync({ tip, birth: tip });
    assert.equal(live.scanned_to, tip);
    assert.equal(requests, 1, `expected a single page, made ${requests}`);
});

await check("pages are the full size the node allows", async () => {
    const tip = 4 * SCAN_PAGE;
    const { requests, live } = await sync({ tip });
    assert.equal(live.scanned_to, tip);
    // Each page overlaps the previous by one block, so a page advances the
    // scan by SCAN_PAGE - 1. Anything materially above this means the loop is
    // asking for less than the node allows.
    const ideal = Math.ceil(tip / (SCAN_PAGE - 1));
    assert.equal(requests, ideal, `expected ${ideal} pages, made ${requests}`);
});

await check("storage is written once per batch, not once per page", async () => {
    const tip = SCAN_BATCH * (SCAN_PAGE - 1);
    const { requests, writes } = await sync({ tip });
    assert.equal(requests, SCAN_BATCH, `expected ${SCAN_BATCH} pages, made ${requests}`);
    assert.ok(writes < requests, `batching must write less often than it fetches: ${writes} writes for ${requests} pages`);
    assert.ok(writes <= 2, `expected at most two writes, made ${writes}`);
});

await check("several pages are on the wire at once", async () => {
    const h = withInfo({ tip: 20 * SCAN_PAGE });
    await syncWallet({ session: h.session, rpc: h.rpc, genesis: GENESIS, assertCurrent: () => {} });
    const { concurrent } = h.stats();
    assert.ok(concurrent > 1, `a scan must not wait one round trip at a time, saw ${concurrent}`);
    assert.ok(concurrent <= SCAN_AHEAD, `read-ahead must stay bounded, saw ${concurrent}`);
});

await check("a page travels while the previous batch is being written", async () => {
    // The win is overlapping the network with the durable write, so the write
    // has to take time for the question to mean anything.
    const h = withInfo({ tip: 6 * SCAN_PAGE });
    let sawInFlightDuringWrite = false;
    const plain = h.session.edit.bind(h.session);
    h.session.edit = async change => {
        // Checked on entry: the point is that the next page was already on the
        // wire when the write began, not that it is still there afterwards.
        if (h.inFlight() > 0) sawInFlightDuringWrite = true;
        await new Promise(resolve => setTimeout(resolve, 5));
        return plain(change);
    };
    await syncWallet({ session: h.session, rpc: h.rpc, genesis: GENESIS, assertCurrent: () => {} });
    assert.ok(sawInFlightDuringWrite, "no page was in flight while the wallet was being written");
});

await check("a short chain is not over-requested", async () => {
    const { requests, live } = await sync({ tip: 3 });
    assert.equal(live.scanned_to, 3);
    assert.equal(live.outputs.length, 4);
    assert.equal(requests, 1);
});

await check("a wallet already at the tip asks for nothing", async () => {
    const tip = 1_000;
    const h = withInfo({ tip });
    h.live.scan_from = tip + 1;
    h.live.scanned_to = tip;
    h.live.anchor = chain(tip + 1).hash(tip);
    const before = h.stats().requests;
    await syncWallet({ session: h.session, rpc: h.rpc, genesis: GENESIS, assertCurrent: () => {} });
    // Only the opening status read and the confirming one; no scan pages.
    assert.equal(h.stats().requests, before, "a caught-up wallet must not fetch pages");
});

console.log(failures ? `\n${failures} failing` : "\nbrowser scan loop ok");
process.exit(failures ? 1 : 0);
