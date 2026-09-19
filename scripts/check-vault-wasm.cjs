#!/usr/bin/env node
// Native -> WASM -> native fixtures only. No network, browser or user storage.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {performance} = require('node:perf_hooks');
const [bindingFile, nativeFile, wasmFile, fundedFile] = process.argv.slice(2);
assert.ok(bindingFile && nativeFile && wasmFile && fundedFile,'Pass generated bindings, native fixture, new WASM fixture, and funded fixture paths.');
globalThis.crypto ??= require('node:crypto').webcrypto;
const {BrowserVault} = require(path.resolve(bindingFile));
const PASSWORD = 'Nightfall public interop test password';
// Public 256-bit zero-entropy BIP39 test vector. Never fund it.
const PHRASE = Array(23).fill('abandon').concat('art').join(' ');
const native = fs.readFileSync(nativeFile);
const vault = new BrowserVault(native);
assert.equal(vault.isLocked(),true);
assert.throws(()=>vault.address(),/locked/i);
assert.throws(()=>vault.unlock('incorrect test password'),/Wrong password or damaged/);
assert.equal(vault.isLocked(),true);
const start = performance.now();
vault.unlock(PASSWORD);
const elapsed = performance.now()-start;
assert.equal(vault.isLocked(),false);
const info = JSON.parse(vault.info());
assert.equal(info.birth_height,42); assert.equal(info.scanned_to,42);
assert.equal(vault.recoveryWords(),PHRASE);
vault.checkRecovery(PHRASE.toUpperCase());
assert.throws(()=>vault.checkRecovery('invalid phrase'),/24 recovery words/);
assert.equal(JSON.parse(vault.balance(42)).total,'0.00000000');
assert.deepEqual(JSON.parse(vault.history()),[]);
assert.throws(()=>vault.balance(NaN),/safe integer/);
assert.throws(()=>vault.ingestPage('[]','null',42),/Invalid spent-output/);
assert.throws(()=>vault.ingestPage('[{}]','[]',99),/Invalid scan output/);
assert.equal(JSON.parse(vault.info()).scanned_to,42);
const sealed = vault.lock();
assert.equal(vault.isLocked(),true);
for (const action of [()=>vault.info(),()=>vault.history(),()=>vault.recoveryWords(),()=>vault.viewKey(),()=>vault.resetScan()]) {
  assert.throws(action,/locked/i);
}
const altered = Uint8Array.from(sealed); altered[altered.length-1]^=1;
const damaged = new BrowserVault(altered);
assert.throws(()=>damaged.unlock(PASSWORD),/Wrong password or damaged/);
damaged.free();
const restored = BrowserVault.restore(PHRASE,PASSWORD,42);
assert.equal(restored.address(),info.address);
assert.equal(restored.recoveryWords(),PHRASE);
// create-new prevents accidentally overwriting another fixture or user file.
fs.writeFileSync(wasmFile,restored.snapshot(),{flag:'wx',mode:0o600});
const previous = restored.snapshot();
restored.changePassword('replacement public test password'); restored.lock();
assert.throws(()=>restored.unlock(PASSWORD),/Wrong password or damaged/);
restored.unlock('replacement public test password');
assert.equal(restored.address(),info.address);
const old = new BrowserVault(previous);old.unlock(PASSWORD);assert.equal(old.address(),info.address);
old.free(); restored.free(); vault.free();
assert.throws(()=>BrowserVault.restore(PHRASE,PASSWORD,-1),/safe integer/);
assert.throws(()=>BrowserVault.fromLegacy('{"v":2}',PASSWORD),/Invalid legacy/);

// ---- preparing a payment must not move the wallet -------------------------
//
// A browser has no filesystem to lean on: storage can be refused, evicted or
// simply fail, and there is no fsync to fall back on. So the wallet must not
// change until the bytes the host was told to store are actually stored.
// `prepareSend` therefore works on a copy and hands back both the transaction
// and that copy's ciphertext; only `commit` adopts it.
//
// The order is enforced by the API rather than by a comment: `commit` consumes
// the prepared object, so `sealed` can only be read beforehand — which is
// exactly when it has to be written.
const funded = new BrowserVault(fs.readFileSync(fundedFile));
funded.unlock(PASSWORD);
const fundedInfo = JSON.parse(funded.info());
assert.equal(fundedInfo.outputs,2,'funded fixture must carry two spendable coins');
assert.equal(fundedInfo.scanned_to,1);
// Above the 1440-block maturity window, so the mined coins can be spent.
const TIP = 5_000, WHEN = 1_800_000_500;
const elsewhere = BrowserVault.create(PASSWORD,0);
const target = elsewhere.address();

const before = funded.info(), beforeHistory = funded.history();
// Two mined coins, so two history rows before any payment exists.
assert.equal(JSON.parse(beforeHistory).length,2);
assert.throws(()=>funded.prepareSend(funded.address(),'1.00000000','',TIP,WHEN),/your own address/);
assert.throws(()=>funded.prepareSend(target,'999999.00000000','',TIP,WHEN),/insufficient/i);
assert.equal(funded.info(),before,'a refused payment must leave the wallet alone');

// Prepared and then thrown away: still nothing moved.
funded.prepareSend(target,'2.00000000','discarded',TIP,WHEN).discard();
assert.equal(funded.info(),before,'discarding a prepared payment must change nothing');
assert.equal(funded.history(),beforeHistory);

const prepared = funded.prepareSend(target,'1.00000000','public fixture',TIP,WHEN);
const preparedTxid = prepared.txid;
assert.equal(preparedTxid.length,64);
assert.equal(prepared.fee,'0.00100000');
assert.ok(JSON.parse(prepared.tx).inputs.length>0);
assert.equal(funded.info(),before,'prepareSend must not move the live wallet');
assert.equal(funded.history(),beforeHistory);

// A candidate belongs to the wallet it came from and to no other.
const stranger = BrowserVault.create(PASSWORD,0);
assert.throws(()=>stranger.commit(funded.prepareSend(target,'1.00000000','',TIP,WHEN)),
  /vault|wallet|network/i);
assert.equal(stranger.history(),'[]','a refused commit must not touch the other wallet');

// Read the bytes the host must store, THEN adopt. This is the whole contract.
const toStore = prepared.sealed;
funded.commit(prepared);
const committed = JSON.parse(funded.history());
assert.equal(committed.length,3,'exactly one row more than the two mined coins');
// record_send inserts at the front, so the payment is the newest row.
assert.equal(committed[0].pending,true);
assert.equal(committed[0].txid,preparedTxid,'the committed entry must be the payment we prepared');
assert.equal(committed[0].memo,'public fixture');
// Amount plus fee: the fee leaves the wallet too, so a balance that showed
// only the amount would tell the owner they still have money they do not.
assert.equal(JSON.parse(funded.balance(TIP)).pending_out,'1.00100000');

// What we were told to store must reopen to exactly what we ended up with.
const reopened = new BrowserVault(toStore);
reopened.unlock(PASSWORD);
assert.equal(reopened.info(),funded.info(),'stored ciphertext must match the committed session');
assert.equal(reopened.history(),funded.history());
reopened.free(); stranger.free(); elsewhere.free(); funded.free();

console.log(`vault WASM ok: native import, lock gates, recovery, corruption, rotation, reverse fixture, and prepare/commit payment isolation; measured unlock ${elapsed.toFixed(0)} ms in Node (not a mobile benchmark)`);
