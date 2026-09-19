#!/usr/bin/env node
// Static guard + isolated self-test: no archive, wallet access, upload or deploy.
//
// The weekly run publishes the site. It no longer does so itself: it calls
// deploy-website.sh, which owns the checks and the deploy. That indirection is
// the point — two lists of "what must be true before the site goes live" drift,
// and the one nobody reads drifts silently. But an indirection is also where a
// guarantee goes to die, so this follows it through instead of trusting it: the
// weekly script must call the deploy script, and the deploy script must deploy.
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';

const script=fileURLToPath(new URL('./weekly-chain-archive.sh',import.meta.url));
const source=readFileSync(script,'utf8');
const deployPath=fileURLToPath(new URL('./deploy-website.sh',import.meta.url));
const deploy=readFileSync(deployPath,'utf8');

// --- the weekly run still publishes the site, via the one gate --------------
assert.match(source,/SITE="\$ROOT\/website"/);
assert.match(source,/scripts\/deploy-website\.sh" --check/,'weekly run must pre-check the site before touching anything');
assert.match(source,/scripts\/deploy-website\.sh"\s*\\?\s*\n?\s*\|\| die/,'weekly run must publish the site through deploy-website.sh');

// --- and that gate is the one doing the work --------------------------------
assert.match(deploy,/cd "\$SITE" && npx wrangler deploy/,'deploy script must actually deploy');
assert.match(deploy,/stamp-cache-busters\.mjs" --check/,'cache busters must be checked, not rewritten, before publishing');
assert.match(deploy,/CHECKS=\(\s*"check-published-channel"/,'published wallet fingerprint must gate every website/weekly deploy');
assert.match(deploy,/cmp -s "\$tmp" "\$SITE\/public\/index\.html"/,'deploy must read the published home page back');

// The two guards that stand between the live site and an older tree.
assert.match(deploy,/github\/website/,'deploy must refuse to publish from the mirror');
assert.match(deploy,/LIVE_PAGES/,'deploy must refuse a tree missing a page the site is serving');

// --- the mirror stays a subset ----------------------------------------------
//
// Anything that only works against the workspace-only website does not belong
// in the public repository: from a fresh clone it cannot run, and a script that
// cannot run is worse than an absent one because it looks available.
// make-chain-bootstrap.sh is deliberately not in this list — it produces the
// archive users download and works from a clone.
assert.doesNotMatch(source,/\$MIRROR|cd github\/website|^git\s/gm);
assert.match(source,/gh release upload/); // Existing bootstrap asset workflow retained.
for (const gone of ['website/','scripts/weekly-chain-archive.sh','scripts/deploy-website.sh']) {
  assert.equal(existsSync(new URL('../github/'+gone,import.meta.url)),false,
    `workspace-only path must not return to the mirror: ${gone}`);
}

// --- the archive ordering still survives a height gaining a digit -----------
const output=execFileSync('bash',[script,'--self-test'],{encoding:'utf8'});
assert.match(output,/self-test ok/);

console.log('bootstrap deploy ok — weekly run publishes through deploy-website.sh');
