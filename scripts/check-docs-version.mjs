#!/usr/bin/env node
/* Version numbers in the shipped docs must be the version being shipped.
 *
 * Written on 30 Aug 2026, after the third release in a row left one behind.
 *
 * The website already had `check-download-links.mjs`, so the site was right
 * every time. Nothing watched README.md, and it drifted quietly: while the
 * tree was on 0.9.0, the "verify your download" instructions still told
 * people to run
 *
 *     shasum -a 256 -c SHA256SUMS-0.8.2.txt
 *
 * — two releases old, and a file that had already been deleted from the
 * downloads page. That is worse than a stale badge. A reader who follows it
 * gets a missing-file error while trying to do the one careful thing we ask
 * of them, and learns that checking is a waste of time.
 *
 * The release badge is not checked here: it reads the latest GitHub release
 * dynamically, so it is right by construction — provided the release is
 * actually published and not left as a draft, which is its own trap and is
 * why it is called out in RELEASING.
 *
 * Historical references are deliberately allowed. "v0.7.0 is a new chain" and
 * "checked against the 0.8.4 code" are statements about the past and must not
 * be rewritten by a release. Only *instructions* — things a reader is meant to
 * type or download — have to name the current version.
 */
import { readFileSync } from "node:fs";

const ROOT = new URL("../", import.meta.url).pathname;

const version = (readFileSync(ROOT + "Cargo.toml", "utf8").match(
    /^version\s*=\s*"([^"]+)"/m,
) || [])[1];
if (!version) {
    console.error("no workspace version in Cargo.toml");
    process.exit(1);
}

/* Files a reader follows, and the patterns that name a downloadable file.
 * Each pattern must match the current version wherever it appears. */
const FILES = ["README.md", "docs/MAINNET.md", "docs/MOBILE.md", "wallets/README.md"];

const PATTERNS = [
    /SHA256SUMS-(\d+\.\d+\.\d+)/g,
    /nightfall-core-(\d+\.\d+\.\d+)/g,
    /nightfall-wallet-(\d+\.\d+\.\d+)/g,
    /nightfalld-(\d+\.\d+\.\d+)/g,
    /NIGHTFALLCOIN-Core-(\d+\.\d+\.\d+)/g,
];

const problems = [];

for (const file of FILES) {
    let text;
    try {
        text = readFileSync(ROOT + file, "utf8");
    } catch {
        continue; // an optional file that does not exist is not a failure
    }
    const lines = text.split("\n");
    for (const pattern of PATTERNS) {
        for (const [i, line] of lines.entries()) {
            for (const m of line.matchAll(pattern)) {
                if (m[1] !== version) {
                    problems.push(
                        `${file}:${i + 1} names ${m[0]} but this tree is ${version}\n` +
                            `    ${line.trim()}`,
                    );
                }
            }
        }
    }
}

if (problems.length) {
    console.error(
        `Documentation still points at an older release.\n` +
            `These are instructions someone will follow, and the files they name are gone:\n`,
    );
    for (const p of problems) console.error("  - " + p);
    process.exit(1);
}

console.log(`docs version ok — every download instruction names ${version}`);
