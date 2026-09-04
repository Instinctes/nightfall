#!/usr/bin/env node
//! Static checks for the browser wallet, no browser required.
//!
//! Three bugs shipped on 26 August 2026 that this would have caught in a
//! second each, and all three were found by a human squinting at a phone:
//!
//!   * a rule on `input[type="text"]` while no field carries a `type`
//!     attribute — every input fell back to the browser default and came out
//!     white and half width;
//!   * a rule on `.words .word` while `wordGrid()` emits classless `<span>`s —
//!     the recovery phrase rendered as bare italic-numbered text;
//!   * a button base scoped to `.actions button`, so a `.primary` on its own
//!     screen got the gradient and neither radius nor padding.
//!
//! Each is the same mistake: a selector written for markup that does not
//! exist. That is exactly what a machine is good at noticing.
//!
//! Deliberately not a test framework and deliberately not a headless browser.
//! It parses two files and compares two sets. Run it before every deploy:
//!
//!     node scripts/check-web-wallet.mjs
//!
//! Exit code 1 on any finding, so CI can gate on it.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const walletDir = join(here, "..", "website", "public", "wallet");
const read = (f) => readFileSync(join(walletDir, f), "utf8");

const app = read("app.js");
const css = read("style.css");
const html = read("index.html");
const sw = read("sw.js");

const problems = [];
const note = (msg) => problems.push(msg);

// ---------------------------------------------------------------- classes ---

// Template literals mean a class attribute can hold `${...}`; those parts are
// skipped rather than guessed at.
const usedClasses = new Set();
for (const m of app.matchAll(/class="([^"]*)"/g)) {
    if (m[1].includes("${")) continue;
    for (const c of m[1].split(/\s+/)) if (c) usedClasses.add(c);
}
for (const c of [...usedClasses].sort()) {
    if (!css.includes(`.${c}`)) note(`class "${c}" is used in app.js and has no rule in style.css`);
}

// -------------------------------------------------------------- selectors ---

// A selector that names an element and an attribute the markup never sets is
// the failure mode above. Check the ones we can check cheaply.
const inputTags = [...app.matchAll(/<input\b([^>]*)>/g)].map((m) => m[1]);
const anyTyped = inputTags.some((a) => /\btype=/.test(a));
if (!anyTyped && /input\[type=/.test(css.replace(/\/\*[\s\S]*?\*\//g, ""))) {
    note(
        "style.css matches on input[type=…] but no <input> in app.js sets a type attribute — " +
            "those rules cannot apply",
    );
}

// ---------------------------------------------------------------- element ---

// Every id the script reaches for must be produced by some template.
const wantedIds = new Set();
for (const m of app.matchAll(/getElementById\("([^"]+)"\)/g)) wantedIds.add(m[1]);
for (const m of app.matchAll(/\$\("#([a-zA-Z0-9_-]+)"\)/g)) wantedIds.add(m[1]);
const producedIds = new Set([...app.matchAll(/id="([a-zA-Z0-9_-]+)"/g)].map((m) => m[1]));
for (const m of html.matchAll(/id="([a-zA-Z0-9_-]+)"/g)) producedIds.add(m[1]);
// Row helpers write `id="${id}"`, so the literal scan above cannot see them.
// Teach the checker about them by name rather than letting it cry wolf — a
// checker that reports four false positives every run is a checker nobody
// reads. Add new helpers here when you write one.
for (const m of app.matchAll(/\bsetRow\(\s*"([a-zA-Z0-9_-]+)"/g)) producedIds.add(m[1]);
for (const id of [...wantedIds].sort()) {
    if (!producedIds.has(id)) note(`app.js looks up #${id}, which no template creates`);
}

// ------------------------------------------------------------------ cache ---

// The cache busters live in four places. Missing one means a returning visitor
// gets the old shell and the bug report describes code that is no longer live.
const vs = new Set([...html.matchAll(/\?v=([a-z0-9-]+)/g)].map((m) => m[1]));
for (const m of app.matchAll(/nightfall_web\.js\?v=([a-z0-9-]+)/g)) vs.add(m[1]);
if (vs.size > 1) {
    note(`cache busters disagree: ${[...vs].join(", ")} — bump all of them together`);
}
// The service worker's cache name has to carry the release, not a counter.
//
// It was "night-wallet-v16", hand-incremented, and this check only asked that
// some number was there. So the 0.9.1 bump could — and on the first attempt
// did — change BUILD in app.js while leaving the cache name alone: the new
// shell is published, every returning wallet keeps serving the old one out of
// its own cache, and nothing anywhere says so. Tying the name to the version
// means the release bump cannot forget it, because this fails.
const release = readFileSync(join(here, "..", "Cargo.toml"), "utf8").match(
    /^version = "([^"]+)"/m,
)?.[1];
const swCache = (sw.match(/const CACHE = "([^"]+)"/) || [])[1];
if (!swCache) {
    note("sw.js has no CACHE name — the old shell will survive a deploy");
} else if (release && !swCache.includes(release)) {
    note(
        `sw.js caches as "${swCache}" but this release is ${release} — ` +
            `returning visitors would keep the old shell. Use "night-wallet-${release}".`,
    );
}

// ------------------------------------------------------------------ build ---

const build = app.match(/const BUILD = "([^"]+)"/)?.[1];
const cargo = readFileSync(join(here, "..", "Cargo.toml"), "utf8").match(/^version = "([^"]+)"/m)?.[1];
if (build && cargo && build !== cargo) {
    note(`BUILD in app.js is ${build} but the workspace is ${cargo}`);
}

// ------------------------------------------------------------------ report --

if (problems.length) {
    console.error(`web wallet: ${problems.length} problem(s)\n`);
    for (const p of problems) console.error(`  - ${p}`);
    process.exit(1);
}
console.log(
    `web wallet ok — ${usedClasses.size} classes, ${wantedIds.size} ids, ` +
        `build ${build}, cache ${[...vs].join("/")}`,
);
