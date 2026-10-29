#!/usr/bin/env node
// Validate the real generated Caddy routes on isolated HTTP loopback only.
import {spawn,execFileSync} from 'node:child_process';
import {mkdtempSync,readFileSync,writeFileSync,rmSync,openSync,closeSync} from 'node:fs';
import {tmpdir} from 'node:os';import {join,resolve} from 'node:path';import assert from 'node:assert/strict';
const root=resolve(new URL('../..',import.meta.url).pathname), caddy=resolve(process.argv[2]||'target/local-tools/caddy/caddy'), kit=join(root,'target/debug/nightfall-nodekit');
const work=mkdtempSync(join(tmpdir(),'nightfall-caddy-'));const port=4200+Math.floor(Math.random()*300), upstream=port+500;
let fixture,proxy;
try{
 fixture=spawn(process.execPath,['scripts/serve-web-wallet-test.mjs',String(upstream)],{cwd:root,stdio:'ignore'});
 const path=join(work,'Caddyfile');
 execFileSync(kit,['selfhost','--domain','wallet.test.invalid','--wallet-dir',join(root,'website/public/wallet-origin'),'--light-port',String(upstream),'--out',path]);
 // Production configuration is validated as generated, without starting TLS
 // or contacting the invalid domain. Runtime routing uses only HTTP loopback.
 execFileSync(caddy,['adapt','--config',path,'--adapter','caddyfile','--validate'],{stdio:'pipe'});
 writeFileSync(path,'{\n admin off\n}\n'+readFileSync(path,'utf8').replace('wallet.test.invalid {',`http://127.0.0.1:${port} {`));
 const fd=openSync(join(work,'caddy.log'),'w');proxy=spawn(caddy,['run','--config',path,'--adapter','caddyfile'],{stdio:['ignore',fd,fd]});closeSync(fd);
 let response;for(let i=0;i<50;i++){try{response=await fetch(`http://127.0.0.1:${port}/`);if(response.ok)break;}catch{}await new Promise(r=>setTimeout(r,100));}
 assert.equal(response?.status,200);assert.match(response.headers.get('content-security-policy'),/connect-src 'self'/);assert.ok((await response.text()).includes('NIGHTFALL'));
 const api=`http://127.0.0.1:${port}/wallet-api`;
 assert.equal((await fetch(api)).status,405);
 const reply=await fetch(api,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({method:'status',params:{},id:1})});assert.equal(reply.status,200);assert.ok(reply.headers.get('cache-control').split(',').map(s=>s.trim()).every(s=>s==='no-store'));assert.equal((await reply.json()).result.tip_height,12);
 assert.equal((await fetch(api,{method:'POST',body:'x'.repeat(600000)})).status,413);
 console.log('Real Caddy: generated config validates; static/CSP, POST-only API, proxy, no-store and size limit passed. Public synthetic upstream, loopback HTTP only.');
}finally{for(const p of [proxy,fixture]){if(p && p.exitCode===null){const done=new Promise(r=>p.once('exit',r));p.kill('SIGTERM');await done;}}rmSync(work,{recursive:true,force:true});}
