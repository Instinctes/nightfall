#!/usr/bin/env node
/* Every download link must name the version that is actually being released.
 *
 * Written on 28 Aug 2026. The site moved to 0.8.3, every .html was updated,
 * and the "Recommended for your device" card kept offering 0.8.2 — because
 * that card is built in js/main.js, which the version bump never touched.
 * The links were not merely stale, they were 404: the 0.8.2 files had just
 * been deleted. A person found it in a screenshot.
 *
 * The first version of this script compared the links against the contents of
 * website/public/downloads/. That worked on the machine that had just filled
 * that directory and failed in CI thirty minutes later, because the same
 * release had *also* removed that directory from git — 60 MB of binaries that
 * belong on the releases page. A check that only runs where the artefacts
 * happen to be lying around is not a check.
 *
 * So the authority here is the workspace version in Cargo.toml. Every
 * versioned download link must carry it. When the directory does happen to be
 * present, the stronger both-directions check runs as well.
 */
import { readFileSync, readdirSync, existsSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const PUB = ROOT + "website/public/";
const DL = PUB + "downloads/";

const version = (readFileSync(ROOT + "Cargo.toml", "utf8").match(
    /^version\s*=\s*"([^"]+)"/m,
) || [])[1];
if (!version) {
    console.error("no workspace version in Cargo.toml");
    process.exit(1);
}

const sources = [];
const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "downloads" || e.name === "pkg") continue;
        const full = dir + e.name;
        if (e.isDirectory()) walk(full + "/");
        else if (/\.(html|js)$/.test(e.name)) sources.push(full);
    }
};
walk(PUB);

const problems = [];
const referenced = new Set();
const LINK = /["'(]\/?downloads\/([A-Za-z0-9._-]+)/g;

for (const file of sources) {
    const text = readFileSync(file, "utf8");
    for (const m of text.matchAll(LINK)) {
        const name = m[1];
        referenced.add(name);
        // Only files whose name carries a version are our business. A link to
        // something unversioned is a different kind of mistake.
        const found = name.match(/\d+\.\d+\.\d+/);
        if (found && found[0] !== version) {
            problems.push(
                `${file.slice(PUB.length)} links downloads/${name}, but this release is ${version}`,
            );
        }
    }
}

if (existsSync(DL)) {
    const have = new Set(readdirSync(DL));
    for (const name of referenced) {
        if (!have.has(name)) problems.push(`downloads/${name} is linked but not present`);
    }
    for (const f of have) {
        if (!referenced.has(f)) problems.push(`downloads/${f} is present but nothing links to it`);
    }
} else {
    console.log("downloads/ not present (it is gitignored) — checking versions only");
}

/* Nothing may ship to the site that is not a deliberate file.
 *
 * On 28 Aug 2026 five `.fuse_hidden…` files were being served live — stale
 * copies of whole pages, left behind when `sed -i` replaced a file that was
 * still open across a FUSE mount. They had no extension, so every check that
 * walks *.html and *.js looked straight past them, and Wrangler happily
 * uploaded all five. One of them answered 200 on the apex.
 */
const ALLOWED_EXT = /\.(html|js|css|json|txt|svg|png|jpg|jpeg|webp|ico|woff2?|wasm|dmg|exe|sh|xml|webmanifest)$/i;
const strays = [];
const sweep = (dir, rel) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        const full = dir + e.name;
        if (e.isDirectory()) {
            sweep(full + "/", rel + e.name + "/");
            continue;
        }
        if (e.name.startsWith(".") || !ALLOWED_EXT.test(e.name)) {
            // Extensionless release binaries are the one legitimate exception.
            if (rel === "downloads/" && !e.name.startsWith(".")) continue;
            strays.push(rel + e.name);
        }
    }
};
sweep(PUB, "");
for (const s of strays) problems.push(`${s} is not a file this site should serve`);

if (problems.length) {
    console.error("download link problems:");
    for (const p of problems) console.error("  - " + p);
    process.exit(1);
}
console.log(`download links ok — ${referenced.size} referenced, all at ${version}`);
