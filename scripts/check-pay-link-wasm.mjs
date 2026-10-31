#!/usr/bin/env node
/* Share links pushed through the real WASM parser, not only the JS transport.
 *
 * `check-pay-link.mjs` covers the envelope: building the fragment, keeping it
 * out of the query, removing it from history, rejecting obvious rubbish. That
 * is the half written in JavaScript. The other half is the shared Rust parser
 * every wallet actually decides with, and a link that survives the envelope
 * still has to face it. This drives that boundary with the bytes a stranger
 * could put in a link.
 *
 * Nothing here signs, reserves, stores or broadcasts anything: a parse is a
 * read. The wallet under test is a throwaway created in memory.
 *
 *     node scripts/check-pay-link-wasm.mjs
 */
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { browserBindings } from './browser-channel.mjs';
const bindings = browserBindings();
const { default: init, BrowserVault } = await import(new URL('nightfall_web.js', bindings));
import { paymentLink, consumePaymentFragment } from "../website/public/wallet-origin/pay-link.js";

await init({
    module_or_path: readFileSync(
        new URL('nightfall_web_bg.wasm', bindings),
    ),
});

const PASSWORD = "public unfunded pay link rehearsal password";
const NOW = 1_800_000_000;

let failures = 0;
const check = (name, body) => {
    try { body(); console.log(`  ok   ${name}`); }
    catch (error) { failures++; console.error(`  FAIL ${name}\n       ${error.message}`); }
};

// A disposable wallet. Its own address is the one thing a payment request may
// never be for, so it doubles as the self-payment fixture.
const vault = BrowserVault.create(PASSWORD, 0);
const mine = vault.address();

/** The fragment a browser would hand the wallet, unwrapped as the app does. */
const throughEnvelope = link => {
    const url = new URL(link);
    let removed = null;
    const history = { replaceState: (_s, _t, next) => { removed = next; } };
    const payload = consumePaymentFragment(
        { href: url.href, hash: url.hash, pathname: url.pathname, search: url.search },
        history,
    );
    return { payload, removed };
};

console.log("pay links through the real parser");

check("a well-formed link survives the envelope and parses to its stated terms", () => {
    const uri = `nightfall:${mine}?network=devnet&amount=125000000&memo=coffee`;
    const { payload, removed } = throughEnvelope(paymentLink(uri));
    assert.equal(payload, uri, "the envelope must return the exact inner URI");
    assert.ok(removed && !String(removed).includes("pay="),
        "the fragment must be dropped from the history entry");
    const read = JSON.parse(vault.readPaymentRequest(payload, NOW));
    assert.equal(read.network, "devnet");
    assert.equal(read.address, mine);
});

check("the request describes its own network rather than being trusted for it", () => {
    // A devnet request read by a mainnet wallet must be described and refused,
    // not silently treated as payable. Inferring the network from an address
    // is exactly the mistake the spec forbids.
    const uri = `nightfall:${mine}?network=devnet&amount=1`;
    const read = JSON.parse(vault.readPaymentRequest(uri, NOW));
    assert.equal(read.network, "devnet");
    assert.ok("payable" in read, "the reader must state whether this wallet may pay it");
});

for (const [name, bad] of [
    ["an empty payload", ""],
    ["a bare address with no scheme", mine],
    ["the wrong scheme", `bitcoin:${mine}?network=devnet`],
    ["no network at all", `nightfall:${mine}?amount=1`],
    ["an unknown field", `nightfall:${mine}?network=devnet&fee=1`],
    ["a duplicated field", `nightfall:${mine}?network=devnet&network=mainnet`],
    ["a fractional amount", `nightfall:${mine}?network=devnet&amount=1.5`],
    ["a negative amount", `nightfall:${mine}?network=devnet&amount=-1`],
    ["an amount in exponent form", `nightfall:${mine}?network=devnet&amount=1e8`],
    ["an amount beyond the supply", `nightfall:${mine}?network=devnet&amount=99999999999999999999`],
    ["a corrupted address", `nightfall:${mine.slice(0, -4)}zzzz?network=devnet`],
    ["a nested share link", `nightfall:${mine}?network=devnet&memo=${encodeURIComponent(paymentLink(`nightfall:${mine}?network=devnet`))}`],
    ["an oversized memo", `nightfall:${mine}?network=devnet&memo=${"m".repeat(4096)}`],
]) {
    check(`refused: ${name}`, () => {
        assert.throws(() => vault.readPaymentRequest(bad, NOW),
            `"${bad.slice(0, 60)}" was accepted`);
    });
}

for (const [name, link] of [
    ["a link to another origin", "https://example.com/#pay=nightfall%3Aabc"],
    ["a link with the payload in the query", `https://wallet.nightfallcoin.org/?pay=${encodeURIComponent(`nightfall:${mine}?network=devnet`)}`],
    ["a fragment that is not a payment", "https://wallet.nightfallcoin.org/#about"],
    // Built by hand: paymentLink() itself refuses to wrap anything that is not
    // a nightfall: URI, so it cannot be used to produce this fixture. That is
    // the right behaviour; this case asks what happens when someone else
    // produces the double wrap.
    ["a doubly wrapped link", "https://wallet.nightfallcoin.org/#pay=" +
        encodeURIComponent("https://wallet.nightfallcoin.org/#pay=" +
            encodeURIComponent(`nightfall:${mine}?network=devnet`))],
]) {
    check(`envelope refuses: ${name}`, () => {
        // The envelope may refuse by returning nothing or by throwing; both
        // stop the link before the parser. Only a payload that gets through
        // needs the second layer.
        let payload;
        try { ({ payload } = throughEnvelope(link)); } catch { return; }
        if (payload === null || payload === undefined || payload === "") return;
        // Anything the envelope lets through must still fail the parser; a
        // wrapper that leaks a non-request onward is the case worth catching.
        assert.throws(() => vault.readPaymentRequest(payload, NOW),
            `"${String(payload).slice(0, 60)}" passed both layers`);
    });
}

check("an expired request is described as expired rather than refused outright", () => {
    const uri = `nightfall:${mine}?network=devnet&amount=1&expires=${NOW - 1}`;
    const read = JSON.parse(vault.readPaymentRequest(uri, NOW));
    assert.ok(read.expired === true || read.payable === false,
        "an expired request must be visible as such to the person reviewing it");
});

check("reading a request leaves the wallet exactly where it was", () => {
    const before = vault.info();
    vault.readPaymentRequest(`nightfall:${mine}?network=devnet&amount=7`, NOW);
    assert.equal(vault.info(), before, "a parse must not move the wallet");
});

vault.free();
console.log(failures ? `\n${failures} failing` : "\npay links through the real parser ok");
process.exit(failures ? 1 : 0);
