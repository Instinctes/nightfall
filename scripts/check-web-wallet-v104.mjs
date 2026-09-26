#!/usr/bin/env node
// Public synthetic fixtures only. No network access or real wallet profiles.
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { WalletSession, checkedPage, checkedStatus, syncWallet } from '../website/public/wallet-origin/session.js';
import init, { BrowserVault } from '../website/public/wallet-origin/pkg/nightfall_web.js';

let count = 0;
async function test(label, run) { await run(); count++; console.log(`  ok ${label}`); }
const fixturePath = new URL('../target/webwallet-test-fixture.json', import.meta.url);
if (!existsSync(fixturePath)) {
  mkdirSync(new URL('../target/', import.meta.url), { recursive: true });
  writeFileSync(fixturePath, execFileSync('cargo', ['+1.98.0', 'run', '--offline', '--locked', '--quiet', '-p', 'nightfall-web', '--example', 'browser_fixture'], {
    cwd: fileURLToPath(new URL('../', import.meta.url)), encoding: 'utf8',
  }));
}
const fixture = JSON.parse(readFileSync(fixturePath));
const wasm = readFileSync(new URL('../website/public/wallet-origin/pkg/nightfall_web_bg.wasm', import.meta.url));
await init({ module_or_path: wasm });
const PASSWORD = 'PUBLIC disposable test password';
const anchor = 'a'.repeat(64);
const funded = () => {
  const vault = BrowserVault.fromLegacy(fixture.state, PASSWORD);
  vault.ingestAnchoredPage(JSON.stringify([fixture.output]), '[]', 0, 0, anchor, anchor);
  return vault;
};
await test('mainnet identity and real WASM recovery', () => {
  assert.equal(BrowserVault.mainnetGenesis(), fixture.genesis);
  const v = BrowserVault.restore(fixture.phrase, PASSWORD, 0);
  assert.equal(v.address(), fixture.address); v.free();
});
await test('anchored scan persists real encrypted outputs and refuses a changed branch', () => {
  const v = funded();
  assert.equal(JSON.parse(v.balance(0)).available, '10.00000000');
  const saved = v.snapshot();
  assert.throws(() => v.ingestAnchoredPage('[]', '[]', 0, 0, 'b'.repeat(64), 'b'.repeat(64)), /chain changed/);
  const reopened = new BrowserVault(saved); reopened.unlock(PASSWORD);
  assert.equal(JSON.parse(reopened.balance(0)).available, '10.00000000');
  assert.equal(JSON.parse(reopened.info()).scan_anchor, anchor);
  reopened.free(); v.free();
});
await test('malformed output and spent encoding cannot advance the saved scan', () => {
  const v = funded();
  for (const output of [{ ...fixture.output, height: 8 }, { ...fixture.output, payload: '00' }, { ...fixture.output, commit: 'bad' }]) {
    assert.throws(() => v.ingestAnchoredPage(JSON.stringify([output]), '[]', 0, 1, anchor, 'b'.repeat(64)));
    assert.equal(JSON.parse(v.info()).scanned_to, 0);
  }
  assert.throws(() => v.ingestAnchoredPage('[]', '["bad"]', 0, 1, anchor, 'b'.repeat(64)));
  v.free();
});
await test('review uses exact decimal arithmetic and UTF-8 memo limits', () => {
  const v = funded();
  const detail = JSON.parse(v.paymentDetails(fixture.recipient, '1,00000001', 'memo'));
  assert.equal(detail.amount, '1.00000001'); assert.equal(detail.total, '1.00100001');
  assert.throws(() => v.paymentDetails(fixture.recipient, '0.000000001', ''));
  assert.throws(() => v.paymentDetails(fixture.recipient, '1', '€'.repeat(22)));
  assert.equal(JSON.parse(v.info()).pending, 0); v.free();
});
function memoryStore() {
  return { writes: [], withWriterLock: run => run(), async save(bytes, { expectSeq }) {
    assert.equal(expectSeq, this.writes.length + 1); this.writes.push(new Uint8Array(bytes)); return expectSeq + 1;
  } };
}
const prepare = candidate => {
  const p = candidate.prepareSend(fixture.recipient, '1', 'test', 0, 1750000000);
  const result = { tx: p.tx, txid: p.txid }; candidate.commit(p); return result;
};
await test('payment is durable before host receives broadcast data; retry is identical', async () => {
  const store = memoryStore(), s = new WalletSession(store); s.open(funded(), 1);
  const payment = await s.edit(prepare);
  assert.equal(store.writes.length, 1);
  const reopened = new BrowserVault(store.writes[0]); reopened.unlock(PASSWORD);
  assert.equal(reopened.pendingTransaction(payment.txid), payment.tx);
  assert.equal(JSON.parse(reopened.info()).pending, 1);
  const restored = BrowserVault.fromBackup(store.writes[0], PASSWORD);
  assert.throws(() => restored.pendingTransaction(payment.txid), /cannot be broadcast/);
  assert.equal(JSON.parse(restored.info()).withheld, 1);
  restored.free(); reopened.free(); s.close();
});
await test('failed save never returns a transaction and closes uncertain session', async () => {
  const store = memoryStore(); store.save = async () => { throw new Error('quota'); };
  const s = new WalletSession(store); s.open(funded(), 1);
  await assert.rejects(s.edit(prepare), /quota/); assert.equal(s.vault, null);
});
await test('locking during a pending save cannot revive the session or return its transaction', async () => {
  const store = memoryStore(); let finish;
  store.save = () => new Promise(resolve => { finish = resolve; });
  const s = new WalletSession(store); s.open(funded(), 1);
  const job = s.edit(prepare); s.close(); finish(2);
  await assert.rejects(job, /locked/); assert.equal(s.vault, null); assert.equal(s.seq, null);
});
await test('invalid payment leaves the original unlocked and unmodified', async () => {
  const store = memoryStore(), s = new WalletSession(store); s.open(funded(), 1);
  await assert.rejects(s.edit(candidate => candidate.prepareSend(fixture.recipient, '100', '', 0, 1750000000)));
  assert.ok(s.vault); assert.equal(store.writes.length, 0); assert.equal(JSON.parse(s.vault.info()).pending, 0); s.close();
});
await test('password change belongs to candidate until verified persistence', async () => {
  const store = memoryStore(), s = new WalletSession(store); s.open(funded(), 1);
  await s.edit(candidate => { candidate.changePassword('PUBLIC replacement password'); });
  const stored = new BrowserVault(store.writes[0]);
  assert.throws(() => stored.unlock(PASSWORD)); stored.unlock('PUBLIC replacement password');
  assert.equal(stored.address(), fixture.address); stored.free(); s.close();
});
await test('rebuild preserves pending reservations and quarantines old payments', async () => {
  const v = funded(); const p = v.prepareSend(fixture.recipient, '1', '', 0, 1750000000); const txid = p.txid; v.commit(p);
  v.beginRescan(); assert.equal(JSON.parse(v.info()).withheld, 1);
  v.ingestAnchoredPage(JSON.stringify([fixture.output]), '[]', 0, 0, anchor, anchor);
  assert.equal(JSON.parse(v.balance(0)).available, '0.00000000');
  assert.throws(() => v.pendingTransaction(txid)); v.free();
});
await test('page and status gates reject partial, foreign, pruned and unsafe ranges', () => {
  const status = { network: 'mainnet', genesis: fixture.genesis, tip_height: 1, blocks: 2, tip: anchor };
  assert.equal(checkedStatus(status, fixture.genesis), status);
  for (const bad of [{ loading: true }, { network: 'devnet' }, { tip_height: 1.2 }, { genesis: anchor }, { blocks: 0 }])
    assert.throws(() => checkedStatus({ ...status, ...bad }, fixture.genesis));
  const page = { genesis: fixture.genesis, from: 0, blocks: 2, scanned_to: 1, tip_height: 1, available_from: 0,
    outputs: [], spent: [], from_hash: anchor, scanned_hash: anchor };
  assert.equal(checkedPage(page, 0, 256, fixture.genesis), page);
  for (const bad of [{ blocks: 0 }, { blocks: 3 }, { from: 1 }, { available_from: 1 }, { scanned_to: 4 }, { outputs: null }, { spent: null }])
    assert.throws(() => checkedPage({ ...page, ...bad }, 0, 256, fixture.genesis));
});
function replacementNode({ pruned = false, moving = false } = {}) {
  const branch = 'b'.repeat(64); let statuses = 0, pages = 0;
  return async (method, params) => {
    assert.ok(['status', 'wallet_scan'].includes(method), 'background repair must never broadcast');
    if (method === 'status') return { network: 'mainnet', genesis: fixture.genesis, blocks: 1,
      tip_height: 0, tip: moving && statuses++ > 0 ? 'c'.repeat(64) : branch };
    pages++; assert.equal(params.from, 0);
    return { genesis: fixture.genesis, from: 0, blocks: 1, scanned_to: 0, tip_height: 0,
      available_from: pruned && pages > 1 ? 1 : 0, from_hash: branch, scanned_hash: branch,
      outputs: [fixture.output], spent: [] };
  };
}
await test('background reorg rebuild completes without input and retains withheld payment inputs', async () => {
  const store = memoryStore(), s = new WalletSession(store), v = funded();
  const payment = prepare(v); s.open(v, 1);
  const status = await syncWallet({ session: s, rpc: replacementNode(), genesis: fixture.genesis, assertCurrent() {} });
  assert.equal(status.tip, 'b'.repeat(64)); assert.equal(store.writes.length, 2);
  assert.equal(JSON.parse(s.vault.info()).withheld, 1);
  assert.equal(JSON.parse(s.vault.balance(0)).available, '0.00000000');
  assert.throws(() => s.vault.pendingTransaction(payment.txid), /cannot be broadcast/);
  s.close();
});
await test('automatic rebuild checks replay availability before changing the saved wallet', async () => {
  const store = memoryStore(), s = new WalletSession(store); s.open(funded(), 1);
  await assert.rejects(syncWallet({ session: s, rpc: replacementNode({ pruned: true }), genesis: fixture.genesis, assertCurrent() {} }), /incomplete/);
  assert.equal(store.writes.length, 0); assert.equal(JSON.parse(s.vault.info()).scan_anchor, anchor);
  assert.equal(JSON.parse(s.vault.balance(0)).available, '10.00000000'); s.close();
});
await test('repeated branch changes stop after one repair and retry on the next background pass', async () => {
  const store = memoryStore(), s = new WalletSession(store); s.open(funded(), 1);
  await assert.rejects(syncWallet({ session: s, rpc: replacementNode({ moving: true }), genesis: fixture.genesis, assertCurrent() {} }), /next background pass/);
  assert.equal(store.writes.length, 2); assert.equal(JSON.parse(s.vault.info()).scan_anchor, 'b'.repeat(64)); s.close();
});
const app = readFileSync(new URL('../website/public/wallet-origin/app.js', import.meta.url), 'utf8');
const html = readFileSync(new URL('../website/public/wallet-origin/index.html', import.meta.url), 'utf8');
await test('host IDs exist and private state never goes to localStorage or remote origins', () => {
  for (const match of app.matchAll(/\$\("([\w-]+)"\)/g)) assert.ok(html.includes(`id="${match[1]}"`), `missing ${match[1]}`);
  assert.doesNotMatch(app, /localStorage|sessionStorage|\bparseFloat\s*\(/);
  assert.match(app, /BrowserVault\.fromBackup/); assert.match(app, /candidate\.prepareSend/);
  assert.equal([...app.matchAll(/fetch\(/g)].length, 1); assert.match(app, /fetch\("\/wallet-api"/);
});
console.log(`Web wallet 1.0.5: ${count} checks passed with real WASM and public synthetic keys; no network.`);
