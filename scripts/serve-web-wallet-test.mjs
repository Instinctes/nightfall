#!/usr/bin/env node
// Local UI fixture. Never forwards requests or touches a real wallet/node.
import http from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { resolve, extname } from 'node:path';
import { createHash } from 'node:crypto';
const root = resolve('website/public/wallet-origin');
const fixture = JSON.parse(readFileSync('target/webwallet-test-fixture.json', 'utf8'));
// Optional branch label lets the same disposable browser profile encounter a
// reorg after restarting this fixture, without modifying any real chain.
const hash = height => createHash('sha256').update(`PUBLIC SYNTHETIC BLOCK ${height}${process.argv[3] || ''}`).digest('hex');
const port = Number(process.argv[2] || 4174);
const tip = 12;
http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${port}`);
  res.setHeader('Cache-Control', 'no-store');
  if (url.pathname === '/wallet-api' && req.method === 'POST') {
    let body = ''; for await (const chunk of req) { body += chunk; if (body.length > 524288) { res.writeHead(413).end(); return; } }
    let call; try { call = JSON.parse(body); } catch { res.writeHead(400).end(); return; }
    let result;
    if (call.method === 'status') result = { network: 'mainnet', genesis: fixture.genesis, tip_height: tip, tip: hash(tip), blocks: tip + 1, peers: 8, mempool: 0, loading: false };
    if (call.method === 'wallet_scan') {
      const from = call.params.from, to = Math.min(tip, from + call.params.limit - 1);
      result = { from, scanned_to: to, blocks: to - from + 1, tip_height: tip, genesis: fixture.genesis,
        available_from: 0, pruned: false, from_hash: hash(from), scanned_hash: hash(to),
        outputs: from === 0 ? [fixture.output] : [], spent: [] };
    }
    res.setHeader('Content-Type', 'application/json');
    // Deliberately fail every submission, exercising persisted-payment recovery.
    res.end(JSON.stringify(result ? { result, id: 1 } : { error: 'Local test: broadcast deliberately disabled.', id: 1 })); return;
  }
  const file = resolve(root, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
  if (!file.startsWith(root + '/') || !existsSync(file)) { res.writeHead(404).end(); return; }
  const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.png': 'image/png' };
  res.setHeader('Content-Type', mime[extname(file)] || 'application/octet-stream');
  res.end(readFileSync(file));
}).listen(port, '127.0.0.1', () => console.log(`PUBLIC SYNTHETIC Webwallet fixture at http://127.0.0.1:${port} — all broadcasts disabled.`));
