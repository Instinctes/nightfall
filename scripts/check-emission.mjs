#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
const read = p => readFileSync(new URL('../' + p, import.meta.url), 'utf8');
const source = read('website/public/js/emission.js');
const { emissionState } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const payload = (height, minted = 600000000) => ({ tip_height:height, blocks:height+1, minted });
assert.equal(emissionState(payload(0)).remaining, 7500000);
assert.equal(emissionState(payload(7499999)).remaining, 1);
assert.equal(emissionState(payload(7499999)).progress, 100);
assert.equal(emissionState(payload(7500000)).era, 1);
assert.equal(emissionState(payload(7500000)).nextReward, 1.5);
assert.equal(emissionState(payload(217500000)).nextReward, 0);
assert.equal(emissionState(payload(225000000)).ended, true);
for (const invalid of [null, {}, {...payload(1),loading:true}, payload(-1),
    payload(1,NaN), payload(1,Infinity), payload(1,-1), payload(1,9000000000000001),
    {...payload(1),blocks:8}]) assert.throws(() => emissionState(invalid));
const html = read('website/public/emission/index.html');
const rows = [...html.matchAll(/<tr(?:\s[^>]*)?>((?:(?!<\/tr>)[\s\S])*)<\/tr>/g)]
    .map(m=>m[1]).filter(s=>s.includes('class="e-era"'));
assert.equal(rows.length, 30);
let cumulative = 0n;
for (let era=0;era<30;era++) {
    const reward = 600000000n >> BigInt(era);
    const cells = [...rows[era].matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map(m=>m[1].replace(/<[^>]+>/g,'').replaceAll(',',''));
    cumulative += reward * 7500000n;
    assert.equal(Number(cells[2]), Number(reward)/1e8, `era ${era} reward`);
    assert.equal(Number(cells[4]), Number(reward*7500000n)/1e8, `era ${era} issuance`);
    assert.equal(Number(cells[5]), Number(cumulative)/1e8, `era ${era} cumulative`);
}
assert.equal(cumulative,8999999925000000n);
for (const [,id] of source.matchAll(/get\('([^']+)'\)/g)) assert.ok(html.includes(`id="${id}"`),id);
assert.match(html, /<script type="module" src="\/js\/emission.js/);
assert.match(source, /Network data unavailable/);
// Exercise DOM updates without a browser session, timer or real network call.
const elements = new Map([...html.matchAll(/id="([^"]+)"/g)].map(([,id]) => [id, {
    textContent:'', dataset:{}, value:0, disabled:false, handlers:{},
    addEventListener(event, handler) { this.handlers[event] = handler; },
}]));
const fakeRows = Array.from({length:30}, () => ({
    here:false, setAttribute() { this.here = true; }, removeAttribute() { this.here = false; },
}));
let networkFails = true;
let networkCalls = 0;
runInNewContext(source.replace('export function', 'function'), {
    document: { getElementById:id=>elements.get(id), querySelectorAll:()=>fakeRows, addEventListener(){} },
    window: { addEventListener(){} }, setInterval(){}, AbortSignal,
    fetch:async () => {
        networkCalls++;
        if (networkFails) throw new Error('offline');
        return { ok:true, json:async () => payload(7500000,4500000300000000) };
    },
});
await new Promise(setImmediate);
assert.equal(elements.get('e-status').dataset.state,'error');
assert.match(elements.get('e-status').textContent,/Network data unavailable/);
assert.equal(elements.get('e-refresh').disabled,false);
networkFails = false;
const refreshed = elements.get('e-refresh').handlers.click();
await elements.get('e-refresh').handlers.click(); // busy guard
await refreshed;
assert.equal(networkCalls,2);
assert.equal(elements.get('e-status').dataset.state,'live');
assert.equal(elements.get('e-era').textContent,'1');
assert.equal(fakeRows.filter(r=>r.here).length,1);
assert.equal(fakeRows[1].here,true);
networkFails = true;
await elements.get('e-refresh').handlers.click();
assert.match(elements.get('e-status').textContent,/Not updating · last success/);
assert.equal(elements.get('e-era').textContent,'1');
assert.equal(elements.get('e-refresh').disabled,false);
console.log('emission ok — 30 exact integer eras, boundaries, invalid data, offline/recovery/stale UI and busy guard');
