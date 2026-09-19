// Filesystem boundary for the web build. No build, deploy, or manifest updates.
import {
  copyFileSync, lstatSync, mkdirSync, mkdtempSync, realpathSync, renameSync, rmSync,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const BINDINGS = ['nightfall_web.js', 'nightfall_web_bg.wasm'];

// realpath alone cannot resolve a not-yet-created output. Resolve the nearest
// existing ancestor, then append missing components to that physical path.
function physicalPath(path) {
  try {
    return realpathSync(path);
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    // A dangling symlink is not a missing directory we may safely create.
    try {
      lstatSync(path);
      throw new Error(`Dangling output link: ${path}`);
    } catch (entryError) {
      if (entryError.code !== 'ENOENT') throw entryError;
    }
    const parent = dirname(path);
    if (parent === path) throw error;
    return join(physicalPath(parent), basename(path));
  }
}

function within(path, parent) {
  const rest = relative(parent, path);
  return rest === '' || (!isAbsolute(rest) && rest !== '..' && !rest.startsWith('../'));
}

function checkFiles(output) {
  for (const name of BINDINGS) {
    const path = join(output, name);
    let stat;
    try { stat = lstatSync(path); } catch (error) {
      if (error.code === 'ENOENT') continue;
      throw error;
    }
    if (!stat.isFile() || stat.nlink !== 1) {
      throw new Error(`Output must be a regular, unlinked file: ${path}`);
    }
  }
}

/// The three places a build may land, each named rather than inferred.
///
/// `dev` is anywhere outside the deployed site. `origin` is the 1.0 wallet's
/// own host — deployed, but not the frozen channel. `publish` is the frozen
/// 0.9.5 channel and nothing else: one exact directory, holding the wallet that
/// people with real balances open today.
///
/// Naming them separately is the point. A single "is this allowed" flag would
/// have to decide what an unfamiliar path means, and the safe answer to that
/// question is always "no" — which is how the 1.0 wallet's own directory ended
/// up unbuildable until this existed.
export const DESTINATIONS = {
  dev: null,
  origin: 'website/public/wallet-origin/pkg',
  publish: 'website/public/wallet/pkg',
};

export function resolveOutput(root, requested, destination = 'dev') {
  if (!Object.hasOwn(DESTINATIONS, destination)) {
    throw new Error(`Unknown destination: ${destination}`);
  }
  if (typeof requested !== 'string' || requested.trim() === '') {
    throw new Error('An output directory is required');
  }
  const physicalRoot = realpathSync(root);
  const output = physicalPath(resolve(physicalRoot, requested));
  const publicDir = physicalPath(join(physicalRoot, 'website/public'));
  const liveDir = physicalPath(join(physicalRoot, DESTINATIONS.publish));

  if (destination === 'dev') {
    // Outside the deployed site entirely — physically…
    if (within(output, publicDir) || within(output, liveDir)) {
      throw new Error(`Refused output in/aliasing the published site: ${requested}`);
    }
    // …and lexically, so a symlinked public subtree cannot make a
    // website/public path acceptable by pointing somewhere else.
    if (within(resolve(physicalRoot, requested), join(physicalRoot, 'website/public'))) {
      throw new Error(`Refused published-site path: ${requested}`);
    }
  } else {
    // A named destination is one exact directory, never a path beneath it.
    const expected = physicalPath(join(physicalRoot, DESTINATIONS[destination]));
    if (output !== expected) {
      throw new Error(`Refused output for --${destination}: ${requested}`);
    }
  }
  // Whatever the destination, the frozen channel is only ever reachable by
  // asking for it by name.
  if (destination !== 'publish' && (output === liveDir || within(output, liveDir))) {
    throw new Error(`Refused output in/aliasing the published channel: ${requested}`);
  }
  checkFiles(output);
  return output;
}

export function installBindings(root, requested, destination, source) {
  const output = resolveOutput(root, requested, destination);
  mkdirSync(output, { recursive: true });
  if (resolveOutput(root, output, destination) !== output) {
    throw new Error('Output directory changed');
  }
  const staging = mkdtempSync(join(output, '.nightfall-web-'));
  try {
    for (const name of BINDINGS) copyFileSync(join(source, name), join(staging, name));
    if (resolveOutput(root, output, destination) !== output) {
      throw new Error('Output directory changed');
    }
    for (const name of BINDINGS) renameSync(join(staging, name), join(output, name));
  } finally {
    // Only this invocation's newly created staging directory is removed.
    rmSync(staging, { recursive: true, force: true });
  }
  return output;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [operation, root, output, destination, source] = process.argv.slice(2);
    if (!['resolve', 'install'].includes(operation) || !root || !output ||
        !Object.hasOwn(DESTINATIONS, destination) ||
        (operation === 'install') !== Boolean(source) ||
        process.argv.length !== (operation === 'install' ? 7 : 6)) {
      throw new Error(
        `Usage: web-wallet-output.mjs resolve|install ROOT OUT ${Object.keys(DESTINATIONS).join('|')} [SOURCE]`);
    }
    console.log(operation === 'resolve'
      ? resolveOutput(root, output, destination)
      : installBindings(root, output, destination, source));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
