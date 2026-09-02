#!/usr/bin/env node
/* Static checks for /chain/.
 *
 * Written for the same reason as check-web-wallet.mjs: in 0.8.1 three screens
 * shipped broken because a stylesheet named markup that does not exist, and a
 * person found all three by squinting at a phone. A class with no rule and an
 * id the script reaches for but the page never renders are both silent — the
 * browser does not complain, it just shows nothing.
 */
import { readFileSync, readdirSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const html = readFileSync(ROOT + "website/public/chain/index.html", "utf8");
const js = readFileSync(ROOT + "website/public/js/chain.js", "utf8");
const shared = readFileSync(ROOT + "website/public/css/style.css", "utf8");
const scoped = (html.match(/<style>([\s\S]*?)<\/style>/) || ["", ""])[1];
const css = shared + "\n" + scoped;

const problems = [];

/* Every id the script looks up must exist in the page. */
const wanted = new Set();
for (const m of js.matchAll(/\$\("([A-Za-z0-9_-]+)"\)/g)) wanted.add(m[1]);
for (const m of js.matchAll(/setText\("([A-Za-z0-9_-]+)"/g)) wanted.add(m[1]);
for (const m of js.matchAll(/width\("([A-Za-z0-9_-]+)"/g)) wanted.add(m[1]);
// The three helpers above were the only shapes this knew about, so a section
// written with plain `getElementById` was invisible to it — five new ids went
// in and the count did not move. A check that silently covers less than you
// think is the thing it was written to prevent.
for (const m of js.matchAll(/getElementById\("([A-Za-z0-9_-]+)"\)/g)) wanted.add(m[1]);
const present = new Set([...html.matchAll(/id="([A-Za-z0-9_-]+)"/g)].map((m) => m[1]));
for (const id of wanted) {
    if (!present.has(id)) problems.push(`script reaches for #${id}, the page never renders it`);
}

/* Every class used must have a rule somewhere. */
const used = new Set();
for (const m of html.matchAll(/class="([^"]+)"/g)) {
    for (const c of m[1].split(/\s+/)) if (c) used.add(c);
}
for (const m of js.matchAll(/className = "([^"]+)"/g)) {
    for (const c of m[1].split(/\s+/)) if (c && !c.includes("+")) used.add(c);
}
for (const c of used) {
    if (!new RegExp(`\\.${c.replace(/[-/\\^$*+?.()|[\]{}]/g, "\\$&")}[\\s,.:{>~+\\[]`).test(css)) {
        problems.push(`class .${c} is used but has no rule`);
    }
}

/* The page must not carry an inline script: the policy is script-src 'self',
 * and an inline handler fails silently in production while working locally. */
if (/<script(?![^>]*\bsrc=)[^>]*>[\s\S]*?\S[\s\S]*?<\/script>/.test(html)) {
    problems.push("inline <script> in the page — the CSP blocks it");
}

/* The cache buster must be the workspace version.
 *
 * chain.js is served with max-age=86400. On 28 Aug 2026 it was changed three
 * times in one afternoon while the page kept pointing at `?v=1`, so every
 * visitor who had opened the page earlier that day was running the first
 * version for another 24 hours — invisible bars and all — while the HTML
 * around it was current. Tying the buster to the version means a release
 * cannot forget to break the cache. */
const wsVersion = (readFileSync(ROOT + "Cargo.toml", "utf8").match(
    /^version\s*=\s*"([^"]+)"/m,
) || [])[1];
for (const m of html.matchAll(/src="\/js\/([a-z-]+\.js)\?v=([^"]+)"/g)) {
    if (m[2] !== wsVersion) {
        problems.push(`${m[1]} is busted with ?v=${m[2]}, but this release is ${wsVersion}`);
    }
}

/* Both endpoints the page depends on must exist in the Worker. */
const worker = readFileSync(ROOT + "website/src/index.js", "utf8");
for (const path of ["/network.json", "/chain.json"]) {
    if (!worker.includes(`"${path}"`)) problems.push(`Worker has no route for ${path}`);
}

/* The bootstrap instructions must cover every platform we ship a wallet for.
 *
 * The first version of that section had macOS and Linux and simply omitted
 * Windows, which is the platform whose users are least likely to work out the
 * datadir path on their own. Nothing failed; the instructions were just
 * missing a third of the audience. So the platforms are checked against the
 * download page rather than against a list typed here — add a Windows build
 * and forget the instructions, and this fails.
 *
 * Each platform is matched by the shape of its own datadir, because that is
 * the part a user cannot guess and the part that would be wrong if someone
 * copied a block and edited only the heading.
 *
 * "What we ship" is read from the download links in the pages rather than from
 * a list in this script: a list here would be one more thing to forget to
 * update.
 *
 * It reads the links and not the directory. The first version listed
 * website/public/downloads/, which is the obvious ground truth and works on
 * any machine that has just built a release — and does not exist at all in a
 * fresh clone, because that directory is gitignored. CI crashed on the
 * readdir. The links are committed, name the same artefacts, and are what a
 * visitor actually gets. */
const linkedArtefacts = [];
(function collect(dir) {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "downloads" || e.name === "pkg" || e.name.startsWith(".")) continue;
        const p = dir + "/" + e.name;
        if (e.isDirectory()) collect(p);
        else if (/\.(html|js)$/.test(e.name)) {
            for (const m of readFileSync(p, "utf8").matchAll(
                /["'(]\/?downloads\/([A-Za-z0-9._-]+)/g,
            )) {
                linkedArtefacts.push(m[1]);
            }
        }
    }
})(ROOT + "website/public");

if (linkedArtefacts.length === 0) {
    problems.push("no download links found anywhere on the site — the platform check has nothing to work from");
}
const shippedFiles = linkedArtefacts.join(" ");
const platforms = [
    ["Windows", /windows/i, /%?APPDATA%?|env:APPDATA/],
    ["macOS", /macos|\.dmg/i, /Library\/Application\\? Support\/nightfall/],
    ["Linux", /linux/i, /\.local\/share\/nightfall/],
];
for (const [name, artefact, mark] of platforms) {
    if (artefact.test(shippedFiles) && !mark.test(html)) {
        problems.push(
            `downloads offers a ${name} build, but the bootstrap instructions ` +
                `never mention its datadir — those users have nowhere to put the file`,
        );
    }
}

/* Hashing tools differ per platform and the wrong one is a dead end.
 *
 * `shasum` is a Perl script that is not guaranteed on a Linux box, and does
 * not exist on Windows at all. Telling every platform to run it, which the
 * first version did, means the verification step silently gets skipped by the
 * people most likely to need it. */
for (const [tool, why] of [
    ["Get-FileHash", "Windows has no shasum"],
    ["sha256sum", "shasum is not guaranteed on Linux"],
]) {
    if (!html.includes(tool)) {
        problems.push(`bootstrap instructions never use ${tool} — ${why}`);
    }
}

/* The nav link must be on every page that has a nav, or the page is orphaned. */
const pages = ["index.html", "emission/index.html", "build/index.html", "audit/index.html", "view-key/index.html", "chain/index.html"];
for (const p of pages) {
    const t = readFileSync(ROOT + "website/public/" + p, "utf8");
    if (t.includes('class="nav-links"') && !t.includes('href="/chain/"')) {
        problems.push(`${p} has a nav but no link to /chain/`);
    }
}

if (problems.length) {
    console.error("chain page problems:");
    for (const p of problems) console.error("  - " + p);
    process.exit(1);
}
console.log(`chain page ok — ${used.size} classes, ${wanted.size} ids`);
