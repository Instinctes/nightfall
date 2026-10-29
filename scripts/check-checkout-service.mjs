import assert from 'node:assert/strict';
import {once} from 'node:events';
import http from 'node:http';
import {mkdtempSync,writeFileSync,chmodSync,symlinkSync,linkSync,unlinkSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createCheckoutServer,invoiceArgs,cliBackend,readTokenFile} from './checkout/devnet-service.mjs';
const work=mkdtempSync(join(tmpdir(),'nightfall-token-check-'));
try {
 const file=join(work,'token');writeFileSync(file,'a'.repeat(64)+'\n',{mode:0o600});
 assert.equal(readTokenFile(file),'a'.repeat(64));
 assert.throws(()=>readTokenFile('relative'));
 const alias=join(work,'alias');symlinkSync(file,alias);assert.throws(()=>readTokenFile(alias));unlinkSync(alias);
 linkSync(file,alias);assert.throws(()=>readTokenFile(file));unlinkSync(alias);
 chmodSync(file,0o644);assert.throws(()=>readTokenFile(file));chmodSync(file,0o600);
 writeFileSync(file,'a'.repeat(129));assert.throws(()=>readTokenFile(file));
 writeFileSync(file,'not a token');assert.throws(()=>readTokenFile(file));
 assert.throws(()=>readTokenFile(work));
} finally {rmSync(work,{recursive:true,force:true});}
const token='a'.repeat(64);let state='not_paid',calls=0,active=0,maxActive=0,fail=false,duplicate=false;
const backend={async create(body){if(duplicate)throw Object.assign(new Error("PRIVATE"),{status:409});calls++;active++;maxActive=Math.max(maxActive,active);await new Promise(r=>setTimeout(r,10));active--;return {reference:body.reference,network:'devnet'};},async status(id){if(fail)throw new Error('PRIVATE diagnostic must not leak');return id==='A-1'?{reference:id,settlement:state}:null;}};
const server=createCheckoutServer({token,backend});server.listen(0,'127.0.0.1');await once(server,'listening');
const url=`http://127.0.0.1:${server.address().port}`;
const headers={authorization:`Bearer ${token}`,'content-type':'application/json'};
try{
 assert.equal((await fetch(url+'/invoices/A-1')).status,401);
 assert.equal((await fetch(url+'/health',{headers})).status,200);
 const page=await fetch(url+'/');assert.equal(page.status,200);assert.match(page.headers.get('content-security-policy'),/script-src 'self'/);
 assert.equal((await fetch(url+'/health',{headers:{...headers,origin:url}})).status,200);
 assert.equal((await fetch(url+'/health',{headers:{...headers,origin:'null'}})).status,403);
 assert.equal((await fetch(url+'/invoices/A-1',{headers:{...headers,origin:'https://evil.invalid'}})).status,403);
 assert.equal(await new Promise((resolve,reject)=>{const req=http.get(url+'/invoices/A-1',{headers:{...headers,host:'evil.invalid'}},res=>{res.resume();resolve(res.statusCode);});req.on('error',reject);}),403);
 const body={reference:'A-1',amount_darks:'300000000'};
 assert.equal((await fetch(url+'/invoices',{method:'POST',headers,body:JSON.stringify({...body,network:'mainnet'})})).status,400);
 assert.equal(calls,0);
 for(const amount of ['0','-1','1.5','9000000000000001',1])assert.throws(()=>invoiceArgs({...body,amount_darks:amount}));
 assert.throws(()=>invoiceArgs({...body,reference:'--send'}));
 for(const field of ['send','seed','rpc','datadir'])assert.throws(()=>invoiceArgs({...body,[field]:'x'}));
 const responses=await Promise.all(Array.from({length:3},()=>fetch(url+'/invoices',{method:'POST',headers,body:JSON.stringify(body)})));
 assert.ok(responses.every(r=>r.status===201));assert.equal(maxActive,1);
 duplicate=true;const conflict=await fetch(url+'/invoices',{method:'POST',headers,body:JSON.stringify(body)});assert.equal(conflict.status,409);assert.match(await conflict.text(),/already exists/);duplicate=false;
 for(const next of ['confirming:1/3','confirmed','not_paid']){state=next;assert.equal((await (await fetch(url+'/invoices/A-1',{headers})).json()).settlement,next);}
 fail=true;const failed=await fetch(url+'/invoices/A-1',{headers});assert.equal(failed.status,503);assert.ok(!(await failed.text()).includes('PRIVATE'));
 assert.equal((await fetch(url+'/send',{method:'POST',headers})).status,404);
 assert.throws(()=>cliBackend({binary:'relative',datadir:'relative',rpc:'127.0.0.1:1'}));
 console.log('Checkout service: authentication/origin/host, validation, serialized operations, reorg status, failure redaction and no-send routes passed. Mock backend; real CLI chain behavior tested separately.');
}finally{server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
