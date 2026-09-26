#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const source = readFileSync(new URL('../website/src/index.js', import.meta.url), 'utf8');
const { default: worker } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const hash = n => n.toString(16).padStart(64, '0');
const before = { first_height: 0, headers: [{ height: 10, hash: hash(10), prev_hash: hash(9) }, { height: 11, hash: hash(11), prev_hash: hash(10) }] };
const page = { from: 10, blocks: 2, scanned_to: 11, tip_height: 11, genesis: hash(0), available_from: 0, outputs: [], spent: [] };
let count = 0, calls = [], responses = [];
const originalFetch = globalThis.fetch;
globalThis.fetch = async (_url, init) => {
  calls.push(JSON.parse(init.body));
  if (!responses.length) throw new Error('unexpected upstream call');
  return new Response(JSON.stringify({ result: responses.shift(), error: null }));
};
async function request(params, host = 'wallet.nightfallcoin.org') {
  return worker.fetch(new Request(`https://${host}/wallet-api`, {
    method: 'POST', body: JSON.stringify({ method: 'wallet_scan', params, id: 7 }),
  }), {}, {});
}
try {
  for (const params of [{}, { from: -1, limit: 1 }, { from: 0.5, limit: 1 }, { from: 0, limit: 513 }, { from: 0, limit: 0 }, { from: Number.MAX_SAFE_INTEGER, limit: 2 }]) {
    assert.equal((await request(params)).status, 400); count++;
  }
  assert.equal(calls.length, 0);
  assert.equal((await request({ from: 10, limit: 2 }, 'nightfallcoin.org')).status, 403); count++;
  responses = [before, page, before]; calls = [];
  // 512 is the node's own get_headers ceiling and therefore the largest page
  // this proxy can bracket; the accepted-range fixture tracks it so a future
  // change to one of them cannot silently pass.
  let response = await request({ from: 10, limit: 512 });
  assert.equal(response.status, 200);
  let body = await response.json(); assert.equal(body.id, 7); assert.equal(body.result.from_hash, hash(10)); assert.equal(body.result.scanned_hash, hash(11));
  assert.deepEqual(calls.map(c => c.method), ['get_headers', 'scan_feed', 'get_headers']);
  assert.equal(calls[1].params.limit, 2); assert.equal(calls[2].params.limit, 2); count++;
  for (const headers of [[], [{ ...before.headers[0], height: 11 }], [before.headers[0], { ...before.headers[1], prev_hash: hash(3) }]]) {
    responses = [{ first_height: 0, headers }]; assert.equal((await request({ from: 10, limit: 2 })).status, 502); count++;
  }
  responses = [{ ...before, first_height: 12 }]; assert.equal((await request({ from: 10, limit: 2 })).status, 502); count++;
  for (const after of [{ ...before, headers: before.headers.slice(0, 1) }, { ...before, headers: [before.headers[0], { ...before.headers[1], hash: hash(55) }] }]) {
    responses = [before, page, after]; assert.equal((await request({ from: 10, limit: 2 })).status, 502); count++;
  }
  responses = [before, { ...page, blocks: 1 }, before];
  assert.equal((await request({ from: 10, limit: 2 })).status, 502); count++;
  console.log(`wallet scan proxy: ${count} range, continuity, reorg and origin checks passed; no real network.`);
} finally { globalThis.fetch = originalFetch; }
