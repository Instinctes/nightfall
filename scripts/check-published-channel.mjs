#!/usr/bin/env node
// Prueft, dass der veroeffentlichte 0.9.5-Wallet-Kanal unveraendert ist.
//
// website/public/wallet/ und website/public/releases.json liegen nicht in Git.
// Ein versehentlicher Schreibvorgang waere dort unwiderruflich und unsichtbar.
// scripts/published-channel.sha256 ist der lokale Anker; diese Datei prueft ihn.
//
//   node scripts/check-published-channel.mjs          -> pruefen, Exit 1 bei Abweichung
//   node scripts/check-published-channel.mjs --print  -> aktuelle Summen ausgeben
//
// Ein Abweichen ist kein Fehler des Skripts. Es bedeutet, dass jemand oder etwas
// den Live-Kanal angefasst hat. Vor jeder Reparatur klaeren, was das war.

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const REQUIRED_PATHS = [
  'website/public/wallet/app.js',
  'website/public/releases.json',
  'website/public/wallet/pkg/nightfall_web.js',
  'website/public/wallet/pkg/nightfall_web_bg.wasm',
];

export function parseManifest(text) {
  const entries = [];
  const seen = new Set();
  text.split('\n').forEach((raw, index) => {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) return;
    const match = line.match(/^([0-9a-f]{64})\s+(\S.*)$/);
    if (!match) {
      throw new Error(
        `published-channel.sha256 Zeile ${index + 1} ist keine Pruefsumme: ${line}`,
      );
    }
    if (!REQUIRED_PATHS.includes(match[2]) || seen.has(match[2])) {
      throw new Error(`Unbekannter oder doppelter Kanalpfad in Zeile ${index + 1}: ${match[2]}`);
    }
    seen.add(match[2]);
    entries.push({ expected: match[1], path: match[2] });
  });
  if (entries.length === 0) {
    throw new Error('published-channel.sha256 enthaelt keine einzige Pruefsumme');
  }
  for (const path of REQUIRED_PATHS) {
    if (!seen.has(path)) throw new Error(`Pflichtpfad fehlt in published-channel.sha256: ${path}`);
  }
  return entries;
}

function digest(absolute) {
  return createHash('sha256').update(readFileSync(absolute)).digest('hex');
}

function main() {
const arguments_ = process.argv.slice(2);
if (arguments_.length > 1 || (arguments_.length === 1 && arguments_[0] !== '--print')) {
  throw new Error('Usage: check-published-channel.mjs [--print]');
}
const entries = parseManifest(readFileSync(join(ROOT, 'scripts', 'published-channel.sha256'), 'utf8'));

if (process.argv.includes('--print')) {
  for (const entry of entries) {
    let actual;
    try {
      actual = digest(join(ROOT, entry.path));
    } catch {
      actual = '<fehlt>'.padEnd(64);
    }
    console.log(`${actual}  ${entry.path}`);
  }
  process.exit(0);
}

const problems = [];
for (const entry of entries) {
  let actual;
  try {
    actual = digest(join(ROOT, entry.path));
  } catch (error) {
    problems.push(
      `${entry.path}: nicht lesbar (${error.code ?? error.message}). ` +
        'Die Datei ist nicht in Git - es gibt keine Kopie, aus der sie ' +
        'wiederhergestellt werden koennte.',
    );
    continue;
  }
  if (actual !== entry.expected) {
    problems.push(
      `${entry.path}: veraendert\n` +
        `    erwartet ${entry.expected}\n` +
        `    gefunden ${actual}`,
    );
  }
}

if (problems.length > 0) {
  console.error('Der veroeffentlichte Wallet-Kanal weicht ab:\n');
  for (const problem of problems) console.error(`  ${problem}\n`);
  console.error(
    'Diese Dateien bedienen Nutzer mit echtem Guthaben. Nicht "reparieren",\n' +
      'bevor geklaert ist, wer sie geschrieben hat. Ein Build-Skript, das\n' +
      'versehentlich dorthin geschrieben hat, ist die wahrscheinlichste Ursache:\n' +
      '  git stash list / git reflog helfen hier NICHT - die Dateien sind untracked.\n' +
      'War die Aenderung beabsichtigt, dann gehoert scripts/published-channel.sha256\n' +
      'im selben Schritt aktualisiert, mit der neuen Version im Kommentarkopf.',
  );
  process.exit(1);
}

console.log(`Veroeffentlichter Kanal unveraendert (${entries.length} Dateien).`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
