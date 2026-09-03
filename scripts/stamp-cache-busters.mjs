#!/usr/bin/env node
/* Make every ?v= on the site the fingerprint of the file it points at.
 *
 *   node scripts/stamp-cache-busters.mjs           rewrite them
 *   node scripts/stamp-cache-busters.mjs --check   fail if any is stale
 *
 * The site serves /js/*.js with max-age=86400, so the query string is the
 * only thing that tells a returning browser to fetch again. Those strings
 * were maintained by hand and drifted the way hand-maintained things do:
 * nav.js sat at ?v=1 on five pages since it was written, and chain.js was
 * pinned to the release version, which does not change when a page is fixed
 * between releases.
 *
 * That is not a tidiness problem. On 3 Sep the chain page's download button
 * overflowed a phone screen; the fix went live, the file on the server was
 * correct, and the browser kept running yesterday's copy because ?v=0.9.0
 * still said 0.9.0. The fix existed and reached nobody who had already
 * visited.
 *
 * A content hash cannot drift. It changes exactly when the file changes, and
 * never when it does not — so an unchanged deploy does not throw away a warm
 * cache either.
 *
 * The browser wallet is left alone: it is a service-worker app with its own
 * scheme, where every buster has to agree and check-web-wallet.mjs enforces
 * that. Two schemes, each internally consistent, is better than one that
 * half-fits both.
 */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync, readdirSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const PUB = ROOT + "website/public/";
const check = process.argv.includes("--check");

const fingerprints = new Map();
function fingerprint(name) {
    if (!fingerprints.has(name)) {
        const body = readFileSync(PUB + "js/" + name);
        fingerprints.set(name, createHash("sha256").update(body).digest("hex").slice(0, 8));
    }
    return fingerprints.get(name);
}

const pages = [];
(function walk(dir, rel = "") {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        // The wallet has its own versioning, and downloads/ is release binaries.
        if (["wallet", "pkg", "downloads"].includes(e.name) || e.name.startsWith(".")) continue;
        if (e.isDirectory()) walk(dir + e.name + "/", rel + e.name + "/");
        else if (e.name.endsWith(".html")) pages.push(rel + e.name);
    }
})(PUB);

const stale = [];
let rewritten = 0;

for (const page of pages) {
    const path = PUB + page;
    const before = readFileSync(path, "utf8");
    const after = before.replace(
        /(src="\/js\/([a-z0-9-]+\.js)\?v=)([^"]*)(")/g,
        (whole, head, name, current, tail) => {
            const want = fingerprint(name);
            if (current !== want) stale.push(`${page}: ${name} is stamped ?v=${current}, content is ${want}`);
            return head + want + tail;
        },
    );
    if (after !== before) {
        if (!check) {
            writeFileSync(path, after);
            rewritten++;
        }
    }
}

if (check) {
    if (stale.length) {
        console.error("cache busters are stale:");
        for (const s of stale) console.error("  - " + s);
        console.error("\nrun: node scripts/stamp-cache-busters.mjs");
        process.exit(1);
    }
    console.log(`cache busters ok — ${pages.length} pages, ${fingerprints.size} scripts`);
} else {
    console.log(
        rewritten
            ? `stamped ${rewritten} page(s) across ${fingerprints.size} scripts`
            : `nothing to do — ${pages.length} pages already current`,
    );
}
