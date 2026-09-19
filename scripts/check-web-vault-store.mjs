#!/usr/bin/env node
/**
 * Boundary tests for the browser vault store. No network, no real browser
 * storage, no wallet files.
 *
 * HONEST LIMIT, stated up front because it decides what these results mean:
 * Node has no IndexedDB and no Web Locks, so both are modelled here. These
 * tests therefore check the store's logic against a *model* of IndexedDB, not
 * against IndexedDB. The model implements the parts the store relies on —
 * `add` refusing an existing key, one transaction committing or aborting
 * whole, a read inside a transaction seeing that transaction's own writes,
 * and the commit happening only once no request is outstanding. Where a real
 * browser differs, these tests will not notice. Verification in real browsers
 * stays an open item in docs/WALLET-1.0.0-PLAN.md and is not replaced by this.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  VaultStore, VaultExistsError, BusyError, StaleWriteError, VerificationError,
  migrateLegacyWallet,
} from '../website/public/wallet-origin/vault-store.js';

let checks = 0;
const ok = (label) => { checks += 1; process.stdout.write(`  ok  ${label}\n`); };

// ---------------------------------------------------------------- the model ---

class FakeRequest {
  constructor() {
    this.result = undefined; this.error = null;
    this.onsuccess = null; this.onerror = null; this.onupgradeneeded = null;
  }
}

/** One transaction over one object store, with staged writes.
 *
 * Jobs run in order; each may enqueue more from its callback, which is how
 * `save` issues its `put` from inside the `get` handler. The transaction
 * commits only when the queue drains — matching the rule that matters here:
 * a compare and the write that depends on it are one atomic step. */
class FakeTransaction {
  constructor(db, mode) {
    this.db = db; this.mode = mode; this.error = null;
    this.staged = new Map();
    this.queue = [];
    this.state = 'active';
    this.oncomplete = null; this.onabort = null; this.onerror = null;
    queueMicrotask(() => this.#drain());
  }
  objectStore() { return new FakeStore(this); }
  view(key) { return this.staged.has(key) ? this.staged.get(key) : this.db.data.get(key); }
  abort() {
    if (this.state !== 'active') return;
    this.state = 'aborted';
    this.staged.clear();
  }
  enqueue(job) { this.queue.push(job); }
  async #drain() {
    while (this.state === 'active' && this.queue.length) {
      const job = this.queue.shift();
      job();
      await Promise.resolve();
    }
    if (this.state === 'active') {
      this.state = 'committed';
      // A staged `undefined` is a delete. Removing the key rather than storing
      // an empty value keeps `add` able to tell "gone" from "present but
      // blank", which is the distinction enrolment after a removal rests on.
      for (const [key, value] of this.staged) {
        if (value === undefined) this.db.data.delete(key);
        else this.db.data.set(key, value);
      }
      this.oncomplete?.();
    } else {
      this.onabort?.();
    }
  }
}

class FakeStore {
  constructor(tx) { this.tx = tx; }
  #run(fn) {
    const request = new FakeRequest();
    this.tx.enqueue(() => {
      try {
        request.result = fn();
        request.onsuccess?.();
      } catch (error) {
        request.error = error;
        let prevented = false;
        request.onerror?.({ preventDefault() { prevented = true; } });
        if (!prevented) this.tx.abort();
      }
    });
    return request;
  }
  get(key) { return this.#run(() => structuredClone(this.tx.view(key))); }
  put(record) {
    return this.#run(() => {
      this.tx.staged.set(record.id, this.tx.db.store(record));
      return record.id;
    });
  }
  delete(key) {
    return this.#run(() => {
      this.tx.staged.set(key, undefined);
      return undefined;
    });
  }
  add(record) {
    return this.#run(() => {
      if (this.tx.view(record.id) !== undefined) {
        const error = new Error('Key already exists in the object store.');
        error.name = 'ConstraintError';
        throw error;
      }
      this.tx.staged.set(record.id, this.tx.db.store(record));
      return record.id;
    });
  }
}

class FakeDatabase {
  constructor() {
    this.data = new Map();
    this.objectStoreNames = { contains: () => true };
    // A device that accepts a write and keeps something else. Rare, and the
    // exact reason a write is read back rather than trusted.
    this.corruptWrites = false;
  }
  store(record) {
    const copy = structuredClone(record);
    if (this.corruptWrites) copy.bytes = Uint8Array.from(copy.bytes, (b) => b ^ 1);
    return copy;
  }
  transaction(_store, mode) { return new FakeTransaction(this, mode); }
  createObjectStore() {}
  close() {}
}

function fakeFactory(db = new FakeDatabase()) {
  return {
    db,
    open() {
      const request = new FakeRequest();
      request.result = db;
      queueMicrotask(() => { request.onupgradeneeded?.(); request.onsuccess?.(); });
      return request;
    },
  };
}

/** Web Locks, modelled: exclusive, `ifAvailable`, released when work ends. */
function fakeLocks() {
  const held = new Set();
  return {
    held,
    async request(name, options, work) {
      if (held.has(name)) {
        if (options.ifAvailable) return work(null);
        throw new Error('the model does not queue; tests use ifAvailable');
      }
      held.add(name);
      try { return await work({ name }); } finally { held.delete(name); }
    },
  };
}

const bytes = (...values) => Uint8Array.from(values);
async function freshStore() {
  const factory = fakeFactory();
  const store = await VaultStore.open({ factory, locks: fakeLocks(), name: 'test' });
  return { store, factory };
}

// ------------------------------------------------------------------- tests ---

// A vault is never replaced by accident. Overwriting is indistinguishable from
// replacing a wallet whose words the owner never wrote down.
{
  const { store, factory } = await freshStore();
  assert.equal(await store.load(), null);
  assert.equal(await store.enroll(bytes(1, 2, 3)), 1);
  const loaded = await store.load();
  assert.deepEqual([...loaded.bytes], [1, 2, 3]);
  assert.equal(loaded.seq, 1);
  await assert.rejects(() => store.enroll(bytes(9, 9, 9)), VaultExistsError);
  assert.deepEqual([...(await store.load()).bytes], [1, 2, 3],
    'a refused enrolment must not have touched the stored wallet');
  assert.equal(factory.db.data.size, 1);
  ok('enrolling twice is refused and changes nothing');
}

// The ordinary write path, and its version counter.
{
  const { store } = await freshStore();
  await store.enroll(bytes(1));
  assert.equal(await store.save(bytes(2), { expectSeq: 1 }), 2);
  assert.equal(await store.save(bytes(3), { expectSeq: 2 }), 3);
  const loaded = await store.load();
  assert.deepEqual([...loaded.bytes], [3]);
  assert.equal(loaded.seq, 3);
  ok('each save advances the version and stores the bytes');
}

// Two tabs. The second holds a version that is no longer current, and its
// write must not land — this is the case that silently lost work before.
{
  const { store } = await freshStore();
  await store.enroll(bytes(1));
  const tabA = 1, tabB = 1; // both read version 1
  assert.equal(await store.save(bytes(10), { expectSeq: tabA }), 2);
  await assert.rejects(
    () => store.save(bytes(20), { expectSeq: tabB }),
    (error) => error instanceof StaleWriteError && error.expected === 1 && error.found === 2,
  );
  const loaded = await store.load();
  assert.deepEqual([...loaded.bytes], [10], "the first tab's work must survive");
  assert.equal(loaded.seq, 2);
  ok('a stale write is refused and the other tab keeps its work');
}

// Saving before enrolling has nothing to compare against, and must not create
// a wallet out of thin air.
{
  const { store } = await freshStore();
  await assert.rejects(
    () => store.save(bytes(1), { expectSeq: 1 }),
    (error) => error instanceof StaleWriteError && error.found === null,
  );
  assert.equal(await store.load(), null);
  ok('saving with nothing stored creates nothing');
}

// Bad input is refused before a transaction is opened.
{
  const { store } = await freshStore();
  await assert.rejects(() => store.enroll(new Uint8Array(0)), TypeError);
  await assert.rejects(() => store.enroll('not bytes'), TypeError);
  await store.enroll(bytes(1));
  await assert.rejects(() => store.save(bytes(2), { expectSeq: 0 }), TypeError);
  await assert.rejects(() => store.save(bytes(2), { expectSeq: 1.5 }), TypeError);
  ok('empty, wrongly typed and unversioned writes are refused');
}

// A write that reports success and stores something else is the failure the
// read-back exists to catch.
{
  const db = new FakeDatabase();
  const store = await VaultStore.open({ factory: fakeFactory(db), locks: fakeLocks(), name: 't' });
  await store.enroll(bytes(1, 2, 3));
  db.corruptWrites = true;
  await assert.rejects(
    () => store.save(bytes(5), { expectSeq: 1 }),
    (error) => error instanceof VerificationError && /differ/.test(error.message),
  );
  // The same check has to guard the very first write too, or a wallet could be
  // created, reported as stored, and be unreadable from the start.
  const fresh = await VaultStore.open({
    factory: fakeFactory(Object.assign(new FakeDatabase(), { corruptWrites: true })),
    locks: fakeLocks(), name: 't',
  });
  await assert.rejects(() => fresh.enroll(bytes(1, 2, 3)), VerificationError);
  ok('a write that stores something else is caught by the read-back');
}

// The lock is advisory between live tabs and is not the safety mechanism on
// its own — but while one tab holds it, another must not start writing.
{
  const locks = fakeLocks();
  const factory = fakeFactory();
  const a = await VaultStore.open({ factory, locks, name: 't' });
  const b = await VaultStore.open({ factory, locks, name: 't' });
  let released;
  const holding = new Promise((resolve) => { released = resolve; });
  const first = a.withWriterLock(() => holding);
  await Promise.resolve();
  await assert.rejects(() => b.withWriterLock(() => 'second'), BusyError);
  released('first');
  assert.equal(await first, 'first');
  assert.equal(await b.withWriterLock(() => 'second'), 'second',
    'the lock must be free again once the work ends');
  ok('a second tab cannot take the writer lock while it is held');
}

// Stealing a lock does not stop the previous holder; it only adds a second
// writer. If this ever appears, the coordination is decorative.
{
  const source = readFileSync(
    new URL('../website/public/wallet-origin/vault-store.js', import.meta.url), 'utf8');
  // Comments discuss both of these at length; the check is about the code.
  const code = source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
  assert.ok(!/\bsteal\s*:/.test(code), 'the store must never steal a Web Lock');
  assert.ok(!/localStorage/.test(code),
    'the store must not reach into localStorage itself; migration is injected');
  assert.ok(/\bsteal\b/.test(source) && /localStorage/.test(source),
    '…and this check is only meaningful while the comments still discuss both, '
    + 'which is what makes the code check non-trivial');
  ok('the store never steals a lock and never touches localStorage directly');
}

// Removal. The store cannot know whether the owner still has their words —
// that gate is in the wallet UI, which checks the phrase against this wallet's
// own address first. What it can refuse is removing a version it was not shown.
{
  const { store, factory } = await freshStore();
  await store.enroll(bytes(1, 2, 3));
  await store.save(bytes(4, 5, 6), { expectSeq: 1 });
  await assert.rejects(() => store.remove({ expectSeq: 1 }), StaleWriteError);
  assert.deepEqual([...(await store.load()).bytes], [4, 5, 6],
    'a refused removal must leave the wallet exactly where it was');
  for (const bad of [0, -1, 1.5, null, undefined, '2']) {
    await assert.rejects(() => store.remove({ expectSeq: bad }), TypeError);
  }
  assert.ok(await store.load(), 'a malformed removal must not have removed anything');
  await store.remove({ expectSeq: 2 });
  assert.equal(await store.load(), null);
  assert.equal(factory.db.data.size, 0, 'the record is gone, not blanked');
  ok('removal refuses a version it was not shown, and removes the one it was');
}

// After a removal the origin is genuinely empty, so a new wallet can be
// enrolled — and starts again at version 1 rather than inheriting a count from
// a wallet that no longer exists.
{
  const { store } = await freshStore();
  await store.enroll(bytes(1));
  await store.save(bytes(2), { expectSeq: 1 });
  await store.remove({ expectSeq: 2 });
  assert.equal(await store.enroll(bytes(7, 7)), 1);
  assert.deepEqual([...(await store.load()).bytes], [7, 7]);
  ok('a fresh wallet can be enrolled after a removal, from version one');
}

// Removing nothing is not success. An owner who is told the wallet was removed
// stops looking after it, so "there was nothing there" must not be reported as
// a completed removal.
{
  const { store } = await freshStore();
  await assert.rejects(() => store.remove({ expectSeq: 1 }), StaleWriteError);
  ok('removing from an empty store is refused rather than reported as done');
}

// A store that accepts the delete and keeps the record must not be believed.
{
  const { store, factory } = await freshStore();
  await store.enroll(bytes(1, 2, 3));
  const realDelete = factory.db.transaction.bind(factory.db);
  factory.db.transaction = (name, mode) => {
    const tx = realDelete(name, mode);
    const store_ = tx.objectStore;
    tx.objectStore = () => {
      const s = store_.call(tx);
      s.delete = () => ({ onsuccess: null, onerror: null }); // accepted, ignored
      return s;
    };
    return tx;
  };
  await assert.rejects(() => store.remove({ expectSeq: 1 }), VerificationError);
  factory.db.transaction = realDelete;
  assert.ok(await store.load(), 'and the wallet is still there, which is why it failed');
  ok('a removal that did not remove anything is reported as a failure');
}

// ---------------------------------------------------------------- migration ---

function migrationHarness({ legacy = 'legacy-wallet-state', sealed = bytes(7, 7) } = {}) {
  const state = { legacy, retired: 0, confirmed: 0, sealed: 0 };
  return {
    state,
    readLegacy: () => state.legacy,
    retireLegacy: () => { state.retired += 1; state.legacy = null; },
    seal: async () => { state.sealed += 1; return sealed; },
    confirm: async () => { state.confirmed += 1; },
  };
}

// The ordinary path, in order: nothing is removed until the new copy is
// stored and independently confirmed.
{
  const { store } = await freshStore();
  const h = migrationHarness();
  const result = await migrateLegacyWallet({ store, ...h });
  assert.deepEqual(result, { migrated: true, resumed: false });
  assert.equal(h.state.retired, 1);
  assert.equal(h.state.legacy, null);
  assert.deepEqual([...(await store.load()).bytes], [7, 7]);
  ok('migration seals, stores, confirms, and only then retires the original');
}

// Interrupted while sealing: the original is untouched and nothing is stored.
{
  const { store } = await freshStore();
  const h = migrationHarness();
  h.seal = async () => { throw new Error('storage refused'); };
  await assert.rejects(() => migrateLegacyWallet({ store, ...h }), /storage refused/);
  assert.equal(h.state.legacy, 'legacy-wallet-state');
  assert.equal(h.state.retired, 0);
  assert.equal(await store.load(), null);
  ok('an interruption before the write leaves the original alone');
}

// Stored but not confirmed: both copies remain, and the original is not
// retired. Losing the only readable copy on a failed read is the one outcome
// that cannot be undone.
{
  const { store } = await freshStore();
  const h = migrationHarness();
  h.confirm = async () => { throw new Error('address mismatch'); };
  await assert.rejects(
    () => migrateLegacyWallet({ store, ...h }),
    (error) => error instanceof VerificationError && /left exactly as it was/.test(error.message),
  );
  assert.equal(h.state.legacy, 'legacy-wallet-state');
  assert.equal(h.state.retired, 0);
  assert.ok(await store.load(), 'the stored copy stays for the next run to check');
  ok('a copy that fails verification never becomes the only copy');
}

// Interrupted between storing and retiring: the next run finds both, confirms
// what is stored, and finishes — without sealing the legacy copy again.
{
  const { store } = await freshStore();
  const first = migrationHarness();
  first.retireLegacy = () => { throw new Error('tab closed'); };
  await assert.rejects(() => migrateLegacyWallet({ store, ...first }), /tab closed/);
  assert.equal(first.state.legacy, 'legacy-wallet-state', 'still readable');

  const resume = migrationHarness();
  const result = await migrateLegacyWallet({ store, ...resume });
  assert.deepEqual(result, { migrated: true, resumed: true });
  assert.equal(resume.state.sealed, 0, 'a resumed migration must not seal again');
  assert.equal(resume.state.confirmed, 1);
  assert.equal(resume.state.retired, 1);
  ok('a migration interrupted before retirement resumes and finishes');
}

// Nothing to do, said plainly rather than by doing something.
{
  const { store } = await freshStore();
  const empty = migrationHarness({ legacy: null });
  assert.deepEqual(await migrateLegacyWallet({ store, ...empty }),
    { migrated: false, reason: 'nothing-to-migrate' });

  await store.enroll(bytes(1));
  const done = migrationHarness({ legacy: null });
  assert.deepEqual(await migrateLegacyWallet({ store, ...done }),
    { migrated: false, reason: 'already-migrated' });
  assert.equal(done.state.retired, 0);
  ok('nothing to migrate and already migrated are both no-ops');
}

// Migration takes the writer lock, so two tabs cannot migrate at once.
{
  const locks = fakeLocks();
  const factory = fakeFactory();
  const a = await VaultStore.open({ factory, locks, name: 't' });
  const b = await VaultStore.open({ factory, locks, name: 't' });
  let release;
  const slow = migrationHarness();
  slow.seal = () => new Promise((resolve) => { release = () => resolve(bytes(7, 7)); });
  const running = migrateLegacyWallet({ store: a, ...slow });
  await Promise.resolve();
  await assert.rejects(() => migrateLegacyWallet({ store: b, ...migrationHarness() }), BusyError);
  release();
  await running;
  ok('a second tab cannot start a migration while one is running');
}

console.log(
  `web vault store ok — ${checks} cases against a model of IndexedDB and Web ` +
  `Locks, not the real ones; real-browser verification remains open`,
);
