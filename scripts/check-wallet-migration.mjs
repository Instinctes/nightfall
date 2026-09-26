#!/usr/bin/env node
// No real wallet data, browser profiles, network or transactions. Exercise the
// read-only origin bridge and its failure paths with an isolated in-memory vault.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { LEGACY_KEYS, readLegacySnapshot, inspectLegacySnapshot, assertUnchanged, createVerifiedExport } from "../website/public/wallet/migrate/migration.js";

let count = 0;
const test = async (label, run) => { await run(); count++; console.log(`  ok ${label}`); };
const TXID = "a".repeat(64);
const transaction = { inputs: [{ commit: [1, 2, 3] }], outputs: [], kernel: { fee: 100000 } };
const sent = { direction: "Sent", txid: TXID, height: null, spent_commits: [[1, 2, 3]], raw: JSON.stringify(transaction) };
const state = (history = []) => JSON.stringify({ v: 1, network: "mainnet", seed: "synthetic-fixture", db: { history, scanned_to: 3 } });
const outbox = () => JSON.stringify([{ txid: TXID, tx: transaction, at: 1 }]);
function storage(raw = state(), queued = null) {
  const values = new Map([[LEGACY_KEYS.wallet, raw], [LEGACY_KEYS.outbox, queued]]);
  return {
    getItem: key => values.get(key) ?? null,
    replace: (key, value) => values.set(key, value),
    setItem() { assert.fail("migration must not write old storage"); },
    removeItem() { assert.fail("migration must not delete old storage"); },
  };
}
const address = "nf-public-test-address";
const walletAddress = raw => { assert.equal(typeof raw, "string"); return address; };
let imported = null;
let freed = 0;
class FakeVault {
  constructor(bytes) { assert.deepEqual([...bytes], [78, 70, 86]); this.reopened = true; this.unlocked = false; }
  static fromLegacy(raw, password) {
    assert.equal(password, "test-password-long"); imported = raw;
    const vault = Object.create(this.prototype); vault.unlocked = true; return vault;
  }
  snapshot() { return new Uint8Array([78, 70, 86]); }
  unlock(password) { assert.equal(password, "test-password-long"); this.unlocked = true; }
  address() { assert.equal(this.unlocked, true); return address; }
  free() { freed++; this.unlocked = false; }
}
const args = store => ({ storage: store, snapshot: readLegacySnapshot(store), password: "test-password-long", BrowserVault: FakeVault, walletAddress });

await test("empty browser is distinguished from malformed wallet", () => {
  assert.equal(readLegacySnapshot(storage(null)).state, null);
  assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage("{")), walletAddress), /unreadable/);
});
await test("no-outbox migration preserves the exact state string", async () => {
  const raw = state().replace('"scanned_to":3', '"scanned_to":9007199254740993');
  const store = storage(raw); const result = await createVerifiedExport(args(store));
  assert.equal(imported, raw); assert.equal(result.address, address); assert.equal(result.pending, 0);
  assert.equal(store.getItem(LEGACY_KEYS.wallet), raw); assert.equal(freed, 2);
});
await test("matching outbox carries unresolved reservations without submitting", async () => {
  const store = storage(state([sent]), outbox()); const result = await createVerifiedExport(args(store));
  assert.equal(result.pending, 1); assert.equal(store.getItem(LEGACY_KEYS.outbox), outbox());
});
await test("transaction comparison tolerates object-key order", () => {
  const reordered = { kernel: transaction.kernel, outputs: transaction.outputs, inputs: transaction.inputs };
  const record = JSON.stringify([{ txid: TXID, tx: reordered, at: 1 }]);
  assert.equal(inspectLegacySnapshot(readLegacySnapshot(storage(state([sent]), record)), walletAddress).pending, 1);
});
await test("missing outbox history stops before encryption", async () => {
  imported = null; await assert.rejects(createVerifiedExport(args(storage(state(), outbox()))), /missing from/);
  assert.equal(imported, null);
});
await test("outbox transaction mismatch does not silently drop a payment", () => {
  const other = JSON.stringify([{ txid: TXID, tx: { ...transaction, outputs: [7] } }]);
  assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage(state([sent]), other)), walletAddress), /different transaction/);
});
await test("pending payment without reservations blocks migration", () => {
  assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage(state([{ ...sent, spent_commits: [] }]))), walletAddress), /no saved input reservations/);
});
await test("malformed or repeated outbox entries are refused", () => {
  for (const value of ["{", "{}", "[null]", JSON.stringify([{ txid: TXID, tx: [] }]), JSON.stringify([{ txid: TXID, tx: transaction }, { txid: TXID, tx: transaction }])]) {
    assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage(state([sent]), value)), walletAddress));
  }
});
await test("unsafe transaction numbers are never compared after rounding", () => {
  const big = '{"inputs":[],"fee":9007199254740993}';
  const raw = state([{ ...sent, raw: big }]);
  const queued = `[{"txid":"${TXID}","tx":${big}}]`;
  assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage(raw, queued)), walletAddress), /unsupported transaction number/);
});
await test("non-mainnet input is refused before key access", () => {
  const raw = state().replace('"mainnet"', '"devnet"');
  assert.throws(() => inspectLegacySnapshot(readLegacySnapshot(storage(raw)), () => assert.fail("unexpected key access")), /mainnet/);
});
await test("original wallet and outbox changes invalidate a snapshot", () => {
  for (const key of Object.values(LEGACY_KEYS)) {
    const store = storage(); const snapshot = readLegacySnapshot(store); store.replace(key, "changed");
    assert.throws(() => assertUnchanged(store, snapshot), /another tab/);
  }
});
await test("a change while yielding to the UI aborts before encryption", async () => {
  const store = storage(); imported = null;
  await assert.rejects(createVerifiedExport({ ...args(store), yieldToUI: async () => store.replace(LEGACY_KEYS.wallet, "changed") }), /another tab/);
  assert.equal(imported, null);
});
await test("encrypted reopen failure cleans up and retains original storage", async () => {
  class BadPasswordVault extends FakeVault { unlock() { throw new Error("authentication failed"); } }
  const store = storage(); const previous = freed;
  await assert.rejects(createVerifiedExport({ ...args(store), BrowserVault: BadPasswordVault }), /authentication failed/);
  assert.equal(freed - previous, 2); assert.equal(store.getItem(LEGACY_KEYS.wallet), state());
});
await test("address mismatch fails verification and cleans up both vaults", async () => {
  class WrongAddressVault extends FakeVault { address() { return this.reopened ? "different-address" : address; } }
  const store = storage(); const previous = freed;
  await assert.rejects(createVerifiedExport({ ...args(store), BrowserVault: WrongAddressVault }), /could not be verified/);
  assert.equal(freed - previous, 2); assert.equal(store.getItem(LEGACY_KEYS.wallet), state());
});
await test("change during encryption refuses the now-stale download", async () => {
  const store = storage();
  class ConcurrentVault extends FakeVault { snapshot() { store.replace(LEGACY_KEYS.outbox, "[]"); return super.snapshot(); } }
  await assert.rejects(createVerifiedExport({ ...args(store), BrowserVault: ConcurrentVault }), /another tab/);
});

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
await test("exporter is read-only, local and offers the canonical destination", () => {
  const app = readFileSync(`${root}/website/public/wallet/migrate/app.js`, "utf8");
  const html = readFileSync(`${root}/website/public/wallet/migrate/index.html`, "utf8");
  const core = readFileSync(`${root}/website/public/wallet/migrate/migration.js`, "utf8");
  assert.doesNotMatch(app + core, /\bfetch\s*\(|\bXMLHttpRequest\b|\bsendBeacon\b|\bpostMessage\s*\(|\bsubmit_tx\b|\.setItem\s*\(|\.removeItem\s*\(|\.clear\s*\(/);
  assert.match(html, /https:\/\/wallet\.nightfallcoin\.org\//);
  assert.match(app, /\.nfv/); assert.match(app, /assertUnchanged/);
  for (const match of app.matchAll(/\$\("([\w-]+)"\)/g)) assert.ok(html.includes(`id="${match[1]}"`), `missing ${match[1]}`);
});
console.log(`wallet migration: ${count} checks passed (model tests; real WASM and browser checks remain separate).`);
