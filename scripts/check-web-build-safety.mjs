#!/usr/bin/env node
// Isolated regressions: fake website, fake build tools, no real build/deploy.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFileSync, existsSync, linkSync, mkdirSync, mkdtempSync, readFileSync,
  realpathSync, rmSync, symlinkSync, unlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { REQUIRED_PATHS, parseManifest } from './check-published-channel.mjs';
import { BINDINGS, installBindings, resolveOutput } from './web-wallet-output.mjs';

const scripts = dirname(fileURLToPath(import.meta.url));
const fixture = realpathSync(mkdtempSync(join(tmpdir(), 'nightfall-web-build-guard-')));
const write = (path, content, options) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content, options);
};
let cases = 0;
try {
  for (const name of ['build-web-wallet.sh', 'web-wallet-output.mjs', 'check-published-channel.mjs']) {
    mkdirSync(join(fixture, 'scripts'), { recursive: true });
    copyFileSync(join(scripts, name), join(fixture, 'scripts', name));
  }
  for (const path of REQUIRED_PATHS) write(join(fixture, path), `public unfunded fixture: ${path}\n`);
  const manifest = REQUIRED_PATHS.map(path =>
    `${createHash('sha256').update(readFileSync(join(fixture, path))).digest('hex')}  ${path}`,
  ).join('\n') + '\n';
  write(join(fixture, 'scripts/published-channel.sha256'), manifest);
  assert.equal(parseManifest(manifest).length, 4); cases++;
  for (const invalid of [
    '', manifest.split('\n').slice(1).join('\n'), manifest + manifest.split('\n')[0],
    manifest.replace(REQUIRED_PATHS[0], '../unrelated-file'),
    manifest.replace(REQUIRED_PATHS[0], '/absolute-path'),
    manifest.replace(/^[0-9a-f]/, 'x'),
  ]) { assert.throws(() => parseManifest(invalid)); cases++; }

  const bin = join(fixture, 'bin');
  const marker = join(fixture, 'cargo-called');
  // A successful guard is permitted to reach this stub, but it cannot build.
  write(join(bin, 'cargo'), '#!/bin/sh\nprintf "%s\\n" "$@" > "$BUILD_GUARD_TEST_MARKER"\nexit 77\n', { mode: 0o700 });
  write(join(bin, 'wasm-bindgen'), '#!/bin/sh\nprintf "wasm-bindgen 0.2.100\\n"\n', { mode: 0o700 });
  const run = (args, accepted = false, input = '') => {
    if (existsSync(marker)) unlinkSync(marker);
    const result = spawnSync('bash', [join(fixture, 'scripts/build-web-wallet.sh'), ...args], {
      cwd: tmpdir(), encoding: 'utf8', input, timeout: 10_000,
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, BUILD_GUARD_TEST_MARKER: marker },
    });
    assert.ifError(result.error);
    assert.equal(existsSync(marker), accepted, `${JSON.stringify(args)}\n${result.stdout}\n${result.stderr}`);
    if (accepted) {
      assert.equal(result.status, 77);
      const invocation = readFileSync(marker, 'utf8');
      for (const required of ['+1.98.0', '--locked', '--offline', '--target-dir']) {
        assert(invocation.split('\n').includes(required));
      }
    } else assert.notEqual(result.status, 0);
    cases++;
  };

  run([], true);
  run(['--out', 'target/new/preview'], true);
  run(['--out', 'website/public-preview'], true); // a sibling is not public/
  run(['--out', 'website/public/new/missing/preview']);
  run(['--out', 'target/../website/public/new/pkg']);
  run(['--out', 'website/public/wallet/pkg']);
  run(['--out', '']);
  run(['--out', 'target/a', '--out', 'target/b']);
  run(['--publish', '--out', 'target/a']);
  run(['--out', 'target/a', '--publish']);
  run(['--publish'], false, 'no\n');
  run(['--publish'], true, 'veroeffentlichen\n');

  // The 1.0 wallet's own host: deployed, but not the frozen channel. Allowed
  // by name, and by name only — every other spelling of it is still refused,
  // and it can never resolve onto the channel.
  run(['--origin'], true);
  run(['--origin', '--out', 'target/a']);
  run(['--out', 'target/a', '--origin']);
  // Two destinations in one command is an author who did not decide, and
  // guessing on their behalf is guessing about the channel.
  run(['--origin', '--publish']);
  run(['--publish', '--origin']);
  run(['--publish', '--publish']);
  run(['--out', 'website/public/wallet-origin/pkg']); // not without the flag
  assert.equal(
    resolveOutput(fixture, 'website/public/wallet-origin/pkg', 'origin'),
    join(fixture, 'website/public/wallet-origin/pkg'),
  ); cases++;
  for (const wrong of [
    'website/public/wallet/pkg',            // the channel, asked for as origin
    'website/public/wallet-origin',         // the parent
    'website/public/wallet-origin/pkg/sub', // a path beneath it
    'target/web-wallet-build',              // an ordinary dev path
  ]) {
    assert.throws(() => resolveOutput(fixture, wrong, 'origin'),
      undefined, `origin must refuse ${wrong}`); cases++;
  }
  // …and the channel still refuses everything that is not itself.
  assert.throws(() => resolveOutput(fixture, 'website/public/wallet-origin/pkg', 'publish'));
  cases++;
  assert.throws(() => resolveOutput(fixture, 'target/x', 'nonsense')); cases++;

  const live = join(fixture, 'website/public/wallet/pkg');
  const liveBefore = BINDINGS.map(name => readFileSync(join(live, name), 'utf8'));
  mkdirSync(join(fixture, 'target'), { recursive: true });
  symlinkSync(live, join(fixture, 'target/live-alias'));
  run(['--out', 'target/live-alias']);
  run(['--out', join(fixture, 'target/live-alias') + '/']);
  symlinkSync(join(fixture, 'website/public'), join(fixture, 'target/public-alias'));
  run(['--out', 'target/public-alias/new/missing/pkg']);
  symlinkSync(join(live, 'missing'), join(fixture, 'target/dangling'));
  run(['--out', 'target/dangling']);
  for (const [kind, link] of [['symlink', symlinkSync], ['hardlink', linkSync]]) {
    const output = join(fixture, 'target', kind);
    mkdirSync(output);
    link(join(live, BINDINGS[0]), join(output, BINDINGS[0]));
    run(['--out', output]);
    assert.throws(() => installBindings(fixture, output, 'dev', live)); cases++;
  }

  const source = join(fixture, 'fresh-bindings');
  for (const name of BINDINGS) write(join(source, name), `new test binding: ${name}`);
  const output = installBindings(fixture, 'target/good/preview', 'dev', source);
  assert.equal(output, resolveOutput(fixture, 'target/good/preview'));
  for (const name of BINDINGS) assert.equal(readFileSync(join(output, name), 'utf8'), `new test binding: ${name}`);
  installBindings(fixture, output, 'dev', source); // replacement of ordinary files
  cases += 2;

  // A late directory swap is rejected by the installer independently of the
  // shell's initial resolution, and cannot write through to the live bindings.
  const late = join(fixture, 'target/late');
  assert.equal(resolveOutput(fixture, late), late);
  symlinkSync(live, late);
  assert.throws(() => installBindings(fixture, late, 'dev', source)); cases++;

  // Missing/edited baseline must stop before an expensive build, not afterwards.
  write(join(fixture, 'scripts/published-channel.sha256'), manifest.split('\n').slice(1).join('\n'));
  run([]);
  write(join(fixture, 'scripts/published-channel.sha256'), manifest);
  write(join(fixture, REQUIRED_PATHS[0]), 'changed public fixture');
  run([]);
  assert.deepEqual(BINDINGS.map(name => readFileSync(join(live, name), 'utf8')), liveBefore);
  console.log(`web build safety ok — ${cases} isolated cases; no real build, wallet, Git, or deploy`);
} finally {
  // Only our fresh OS-temp fixture is removed, never a repository directory.
  rmSync(fixture, { recursive: true, force: true });
}
