#!/usr/bin/env node
/* Every download link must point at a file that is actually there.
 *
 * Written on 28 Aug 2026. The site moved to 0.8.3, every .html was updated,
 * and the "Recommended for your device" card kept offering 0.8.2 — because
 * that card is built in js/main.js, which the version bump never touched.
 * The links were not merely stale, they were 404: the 0.8.2 files had just
 * been deleted. A person found it in a screenshot.
 *
 * Nothing here needs the network. The links are strings, the files are on
 * disk, and comparing the two is a millisecond.
 */
import { readFileSync, readdirSync, existsSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;
const PUB = ROOT + "website/public/";
const DL = PUB + "downloads/";

if (!existsSync(DL)) {
    console.error("website/public/downloads/ does not exist");
    process.exit(1);
}
const have = new Set(readdirSync(DL));

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
for (const file of sources) {
    const text = readFileSync(file, "utf8");
    for (const m of text.matchAll(/["'(]\/?downloads\/([A-Za-z0-9._-]+)/g)) {
        if (!have.has(m[1])) {
            problems.push(`${file.slice(PUB.length)} links downloads/${m[1]}, which is not there`);
        }
    }
}

/* And the other direction, so a stale file cannot sit on the site unreferenced
 * after a version bump — that is how 0.7.2 binaries survived three releases. */
const referenced = new Set();
for (const file of sources) {
    for (const m of readFileSync(file, "utf8").matchAll(/["'(]\/?downloads\/([A-Za-z0-9._-]+)/g)) {
        referenced.add(m[1]);
    }
}
for (const f of have) {
    if (!referenced.has(f)) problems.push(`downloads/${f} is on the site but nothing links to it`);
}

if (problems.length) {
    console.error("download link problems:");
    for (const p of problems) console.error("  - " + p);
    process.exit(1);
}
console.log(`download links ok — ${have.size} files, all referenced, all present`);
