// Real generated WASM API and public synthetic fixtures only. No network/storage.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
globalThis.crypto ??= require('node:crypto').webcrypto;
const [bindings, fixturePath, version] = process.argv.slice(2);
const api = require(path.resolve(bindings));
const fixture = JSON.parse(fs.readFileSync(fixturePath, 'utf8'));
assert.equal(api.buildVersion(), version);
assert.equal(api.maxBackupBytes(), 64 * 1024 * 1024);
for (const key of ['create_wallet', 'restore_wallet', 'wallet_address', 'wallet_phrase',
  'wallet_view_key', 'wallet_scan_from', 'wallet_info', 'reset_scan', 'ingest_page',
  'wallet_balance', 'wallet_history', 'build_send']) assert.equal(key in api, false, key);
for (const key of ['BrowserVault', 'requestQrSvg', 'address_qr_svg', 'probe_crypto'])
  assert.equal(typeof api[key], 'function', key);
const {BrowserVault} = api;
const password = 'PUBLIC browser module test password';
const restored = BrowserVault.restore(fixture.phrase, password, 0);
assert.equal(restored.address(), fixture.address);
const bytes = restored.snapshot();
assert.throws(() => BrowserVault.restore('x'.repeat(1025), password, 0), /1024/);
const words = fixture.phrase.split(' '); words[4] = 'PRIVATE_CANARY_NOT_A_REAL_WORD';
assert.throws(() => BrowserVault.restore(words.join(' '), password, 0), e => {
  assert.doesNotMatch(String(e), /PRIVATE_CANARY/); return true;
});
assert.throws(() => new BrowserVault(new Uint8Array(64 * 1024 * 1024 + 1)), /size|limit|large/i);
assert.throws(() => BrowserVault.fromLegacy('x'.repeat(64 * 1024 * 1024), password), /too large for this browser/);
const reopened = new BrowserVault(bytes);
assert.throws(() => reopened.unlock('incorrect public password'));
assert.equal(reopened.isLocked(), true);
reopened.unlock(password); assert.equal(reopened.address(), fixture.address);
assert.deepEqual(reopened.snapshot(), bytes);
const damaged = bytes.slice(); damaged[damaged.length - 1] ^= 1;
const bad = new BrowserVault(damaged);
assert.throws(() => bad.unlock(password)); assert.equal(bad.isLocked(), true);
bad.free(); reopened.free(); restored.free();
console.log('PASS: compiled version, 64 MiB target bound, export surface, recovery redaction, wallet identity, encrypted roundtrip and corruption refusal');
