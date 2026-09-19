#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
const root = new URL('../', import.meta.url);
const read = p => readFileSync(new URL(p,root),'utf8');
const current = read('website/public/audit/index.html');
const archive = read('website/public/audit/2026-08-16/index.html');
assert.match(current,/AI-assisted internal review/);
assert.match(current,/277 passed · 9 explicitly ignored/);
assert.match(current,/Mainnet atomic swaps remain disabled/);
assert.match(current,/not a fresh full-workspace test run/);
assert.match(archive,/Historical review/);
assert.equal(read('docs/AUDIT-2026-09-08.md'),read('website/public/audit/review-2026-09-08.txt'));
const publicRoot = new URL('website/public/',root);
let checkedLinks=0, checkedPages=0;
function crawl(directory) {
    for (const entry of readdirSync(directory,{withFileTypes:true})) {
        if (['wallet','downloads'].includes(entry.name)) continue;
        const path = new URL(entry.name + (entry.isDirectory()?'/':''),directory);
        if (entry.isDirectory()) { crawl(path); continue; }
        if (!entry.name.endsWith('.html')) continue;
        const html = readFileSync(path,'utf8');
        checkedPages++;
        const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(m=>m[1]);
        assert.equal(new Set(ids).size,ids.length,`duplicate ids: ${path.pathname}`);
        for (const [,href] of html.matchAll(/\bhref="([^"]+)"/g)) {
            if (/^(https?:|mailto:|data:)/.test(href)) continue;
            const target = new URL(href, 'https://test.invalid/' + path.pathname.slice(publicRoot.pathname.length));
            if (['/network.json','/supply','/chain.json','/peers','/discord.json'].includes(target.pathname)) {
                assert.ok(read('website/src/index.js').includes(`url.pathname === "${target.pathname}"`));
                checkedLinks++;
                continue;
            }
            let file = new URL('.'+target.pathname,publicRoot);
            if (target.pathname.endsWith('/')) file = new URL('index.html',file);
            assert.ok(existsSync(file),`missing local link ${href} in ${path.pathname}`);
            if (target.hash && file.pathname.endsWith('.html')) {
                assert.ok(readFileSync(file,'utf8').includes(`id="${decodeURIComponent(target.hash.slice(1))}"`),
                    `missing anchor ${href} in ${path.pathname}`);
            }
            checkedLinks++;
        }
    }
}
crawl(publicRoot);
console.log(`security review ok — scope, archive, identical full report; ${checkedLinks} local links across ${checkedPages} pages`);
