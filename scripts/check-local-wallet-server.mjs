#!/usr/bin/env node
// Disposable files and loopback fixtures only; no wallet/node is opened.
import assert from 'node:assert/strict';
import http from 'node:http';
import {once} from 'node:events';
import {spawn} from 'node:child_process';
import {mkdtempSync,mkdirSync,writeFileSync,symlinkSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
const work=mkdtempSync(join(tmpdir(),'nightfall-local-web-'));
const root=join(work,'static');mkdirSync(root);writeFileSync(join(root,'index.html'),'PUBLIC synthetic fixture');
writeFileSync(join(work,'outside.txt'),'DO NOT SERVE');symlinkSync(join(work,'outside.txt'),join(root,'escape.txt'));
let calls=0;
const upstream=http.createServer((req,res)=>{calls++;res.setHeader('Content-Type','application/json');res.end('{"result":{"network":"mainnet"}}');});
upstream.listen(0,'127.0.0.1');await once(upstream,'listening');
const reservation=http.createServer();reservation.listen(0,'127.0.0.1');await once(reservation,'listening');const port=reservation.address().port;await new Promise(r=>reservation.close(r));
const child=spawn(process.execPath,[resolve('scripts/serve-local-wallet.mjs'),root,String(port),`http://127.0.0.1:${upstream.address().port}/`],{stdio:['ignore','pipe','pipe']});
const gone=once(child,'exit');
try{
 await Promise.race([once(child.stdout,'data'),gone.then(()=>{throw new Error('Server exited');}),new Promise((_,reject)=>setTimeout(()=>reject(new Error('Startup timeout')),5000).unref())]);
 const base=`http://127.0.0.1:${port}`;
 let r=await fetch(base+'/');assert.equal(await r.text(),'PUBLIC synthetic fixture');assert.equal(r.headers.get('cache-control'),'no-store');assert.match(r.headers.get('content-security-policy'),/frame-ancestors 'none'/);
 assert.equal((await fetch(base+'/escape.txt')).status,404);
 assert.equal((await fetch(base+'/wallet-api')).status,405);
 assert.equal((await fetch(base+'/',{headers:{Origin:'https://foreign.invalid'}})).status,403);
 const hostile=await new Promise((resolve,reject)=>{const q=http.get(base+'/',{headers:{Host:'foreign.invalid'}},r=>{r.resume();resolve(r.statusCode);});q.on('error',reject);});assert.equal(hostile,403);
 const post=body=>fetch(base+'/wallet-api',{method:'POST',headers:{'Content-Type':'application/json'},body});
 assert.equal((await post('{"method":"export_seed"}')).status,403);assert.equal(calls,0);
 assert.equal((await post('x'.repeat(524289))).status,413);assert.equal(calls,0);
 r=await post('{"method":"status"}');assert.equal(r.status,200);assert.equal((await r.json()).result.network,'mainnet');assert.equal(calls,1);
 console.log('Local serving checks passed: static/CSP, symlink boundary, methods, host/origin, request bounds, loopback proxy.');
}finally{child.kill('SIGTERM');await gone;upstream.closeAllConnections();await new Promise(r=>upstream.close(r));rmSync(work,{recursive:true,force:true});}
