#!/usr/bin/env node
// Local Worker boundary tests only. Never contact a seed or submit a transaction.
import assert from 'node:assert/strict';
import { existsSync, readFileSync, statSync } from 'node:fs';
const source = readFileSync(new URL('../website/src/index.js', import.meta.url), 'utf8');
const { default: worker } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const origin = 'https://nightfallcoin.org';
let calls = 0;
const previous = globalThis.fetch;
globalThis.fetch = async () => { calls++; return new Response('{"result":{"ok":true}}'); };
// A stub that answers 200 for every path hides the thing most worth checking:
// which paths the Worker even looks for. Resolve against the real ./public
// tree the way Cloudflare's asset server does — directory means index.html —
// so "this endpoint is not on the wallet origin" is a fact about the tree and
// not an artefact of the stub.
const PUBLIC = new URL('../website/public', import.meta.url);
const env = { ASSETS: { fetch: async request => {
    let pathname = decodeURIComponent(new URL(request.url).pathname);
    // Cloudflare's asset server normalises `/dir/index.html` to `/dir/` with a
    // 307 rather than serving it. The first deploy of the wallet origin
    // forwarded exactly that redirect to the browser — pointing at an internal
    // path that resolves to nothing on that host — because this stub answered
    // 200 and nothing here modelled the redirect. It does now.
    if (pathname.endsWith('/index.html')) {
        return new Response(null, {
            status: 307,
            headers: { Location: pathname.slice(0, -'index.html'.length) },
        });
    }
    if (pathname.endsWith('/')) pathname += 'index.html';
    const file = new URL('.' + pathname, PUBLIC + '/');
    const found = !pathname.includes('missing') && existsSync(file) && statSync(file).isFile();
    return new Response(found ? 'asset' : 'not found', { status: found ? 200 : 404 });
} } };
const ctx = { waitUntil() {} };
let checks = 0;
async function request(path, options, expected) {
    const response = await worker.fetch(new Request(origin + path, options), env, ctx);
    assert.equal(response.status, expected, `${path}: expected ${expected}`);
    assert.ok(response.headers.get('content-security-policy'), 'CSP');
    checks++;
    return response;
}
try {
    for (const body of ['null', '[]', 'true', '"status"', '{}', '{', '{"method":2}']) {
        await request('/wallet-api', { method: 'POST', body }, 400);
    }
    await request('/wallet-api', { method: 'POST', body: JSON.stringify({method:'mine_one'}) }, 403);
    await request('/wallet-api', { method: 'GET' }, 405);
    await request('/wallet-api', { method: 'POST', body: 'x'.repeat(512 * 1024 + 1) }, 413);
    await request('/wallet-api', { method: 'POST', body: 'é'.repeat(300 * 1024) }, 413);
    const stream = new ReadableStream({ start(controller) {
        controller.enqueue(new Uint8Array(300 * 1024));
        controller.enqueue(new Uint8Array(300 * 1024));
        controller.close();
    } });
    await request('/wallet-api', { method: 'POST', body: stream, duplex: 'half' }, 413);
    assert.equal(calls, 0, 'invalid requests must never reach a seed');
    await request('/wallet-api', { method: 'POST', body: '{"method":"status"}' }, 200);
    assert.equal(calls, 1);
    const home = await request('/', {}, 200);
    assert.ok(!home.headers.get('content-security-policy').includes('wasm-unsafe-eval'));
    const wallet = await request('/wallet/', {}, 200);
    assert.ok(wallet.headers.get('content-security-policy').includes('wasm-unsafe-eval'));
    assert.equal(wallet.headers.get('cache-control'), 'no-store');
    const missing = await request('/downloads/missing.exe', {}, 404);
    assert.equal(missing.headers.get('cache-control'), 'no-store');
    assert.equal(missing.headers.get('content-disposition'), null);
    assert.equal(home.headers.get('x-content-type-options'), 'nosniff');

    // ---- the wallet's own origin -------------------------------------------
    //
    // A browser separates stored keys by origin and by nothing smaller, so the
    // separation is only real if this host is genuinely answered as itself.
    // The failure that looks like success is a 301 back to the apex: the
    // wallet keeps working and keeps sharing storage with the whole site.
    const WALLET_ORIGIN = 'https://wallet.nightfallcoin.org';
    async function walletRequest(path, options, expected) {
        const response = await worker.fetch(
            new Request(WALLET_ORIGIN + path, options), env, ctx,
        );
        assert.equal(response.status, expected, `wallet origin ${path}: expected ${expected}`);
        checks++;
        return response;
    }

    const walletRoot = await walletRequest('/', {}, 200);
    assert.equal(walletRoot.headers.get('location'), null,
        'the wallet origin must never answer with a redirect — not to the apex, '
        + 'and not to the internal directory its files are mounted on');
    // The long spelling of the root is answered here, with a Location this
    // origin actually has — never the asset layer's internal mount point.
    const byName = await walletRequest('/index.html', {}, 308);
    assert.equal(byName.headers.get('location'), '/');
    assert.ok(walletRoot.headers.get('content-security-policy').includes('wasm-unsafe-eval'),
        'the wallet origin needs the wasm policy at its root, not only under /wallet');
    assert.equal(walletRoot.headers.get('cache-control'), 'no-store');
    assert.equal(walletRoot.headers.get('cross-origin-resource-policy'), 'same-origin');
    assert.equal(walletRoot.headers.get('x-frame-options'), 'DENY');

    // Everything the site grew must be absent here. Each of these is an
    // endpoint that cannot be turned against the host holding someone's keys.
    // /discord.json matters most: it answers with `Access-Control-Allow-Origin: *`.
    for (const path of ['/discord.json', '/supply', '/peers', '/network.json', '/chain.json']) {
        const off = await walletRequest(path, {}, 404);
        assert.equal(off.headers.get('access-control-allow-origin'), null,
            `${path} must not hand out a CORS grant on the wallet origin`);
    }
    // Downloads are the site's job. A binary offered by the wallet host would
    // be a binary a phisher only has to get served once.
    const noDownload = await walletRequest('/downloads/nightfall-core.exe', {}, 404);
    assert.equal(noDownload.headers.get('content-disposition'), null);
    await walletRequest('/', { method: 'POST' }, 405);

    // Climbing out of the wallet directory. `new URL()` does not decode `%2F`,
    // so a prefix-and-normalise mapping leaves the escape intact until a later
    // layer decodes it — and `/..%2Fwallet%2Fapp.js` really did serve the
    // published 0.9.5 wallet from this origin while this was being written.
    // That is the one outcome the separation exists to prevent: the old
    // wallet's code running on the new origin, against the new origin's
    // storage. These stay as tests because the bug was not theoretical.
    for (const path of [
        '/..%2Fwallet%2Fapp.js',
        '/%2e%2e%2fwallet%2fapp.js',
        '/%2E%2E/wallet/app.js',
        '/..\\wallet\\app.js',
        '//wallet/app.js',
        '/a%00.js',
        '/wallet-origin/index.html',
    ]) {
        await walletRequest(path, {}, 404);
    }
    // …while ordinary paths still resolve, so the guard is a filter and not
    // simply a closed door. A page is addressed as a directory: Cloudflare
    // rewrites `/name.html` to `/name` with a 307, which this origin refuses
    // rather than forwards, so the extension form does not work here at all.
    await walletRequest('/favicon-32.png', {}, 200);
    await walletRequest('/vault-store.js', {}, 200);
    await walletRequest('/storage-check/', {}, 200);
    // The retired preview cannot mutate the production vault with old logic.
    const preview = await walletRequest('/preview/', {}, 308);
    assert.equal(preview.headers.get('location'), '/');
    await walletRequest('/preview/app.js', {}, 410);
    await walletRequest('/app.js', {}, 200);
    await walletRequest('/session.js', {}, 200);
    await walletRequest('/style.css', {}, 200);
    await walletRequest('/pkg/nightfall_web.js', {}, 200);
    await walletRequest('/pkg/nightfall_web_bg.wasm', {}, 200);
    await walletRequest('/storage-check.html', {}, 404);
    // A directory that is not there is still just missing, not an escape.
    await walletRequest('/nothing-here/', {}, 404);

    // The proxy has to be same-origin here, or the wallet needs CORS — and a
    // CORS grant is an invitation for every other page to use it too.
    const before = calls;
    await walletRequest('/wallet-api', { method: 'POST', body: '{"method":"status"}' }, 200);
    assert.equal(calls, before + 1, 'the wallet origin must reach the seed');
    await walletRequest('/wallet-api', { method: 'POST', body: '{"method":"mine_one"}' }, 403);

    // …and the same files must not also be reachable on the apex. Two origins
    // serving one wallet is one spelling too many to check against a phishing
    // domain, and the second has no storage isolation at all.
    for (const path of ['/wallet-origin/', '/wallet-origin/index.html']) {
        const leaked = await request(path, {}, 404);
        assert.equal(leaked.headers.get('cache-control'), 'no-store');
    }
    // The published 0.9.5 channel is untouched by all of this: an existing
    // holder's seed lives in that origin's storage and nothing else can read it.
    await request('/wallet/', {}, 200);

    console.log(`website security ok — ${checks} local request cases, zero real network requests`);
} finally { globalThis.fetch = previous; }
