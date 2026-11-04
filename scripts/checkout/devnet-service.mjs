#!/usr/bin/env node
// Local Devnet-only adapter. No payment or signing command is exposed.
import http from 'node:http';
import {existsSync, readFileSync, lstatSync, fstatSync, openSync, readSync, closeSync, constants, realpathSync} from 'node:fs';
import {resolve, isAbsolute, join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {createHash, timingSafeEqual} from 'node:crypto';
const execute = promisify(execFile);
const uiRoot=fileURLToPath(new URL('./ui/',import.meta.url));
const MAX_AMOUNT = 9000000000000000n;
const reference = value => typeof value === 'string' && /^[A-Za-z0-9][A-Za-z0-9_-]{0,47}$/.test(value);
const digest = value => createHash('sha256').update(value).digest();
// Bound the read and reject aliases before consuming an authentication secret.
export function readTokenFile(path) {
  if (typeof path!=='string' || !isAbsolute(path)) throw new Error('Provide an absolute token-file path.');
  const before=lstatSync(path);
  const privateFile=meta=>meta.isFile() && meta.nlink===1 && meta.size<=128 &&
    (meta.mode & 0o077)===0 && (typeof process.getuid!=='function' || meta.uid===process.getuid());
  if(!privateFile(before))throw new Error('Token must be a bounded owner-only regular file with one link.');
  const fd=openSync(path,constants.O_RDONLY | (constants.O_NOFOLLOW || 0));
  try {
    const opened=fstatSync(fd);
    if(!privateFile(opened) || opened.dev!==before.dev || opened.ino!==before.ino)throw new Error('Token file changed during open.');
    const bytes=Buffer.alloc(129);
    try {
      const count=readSync(fd,bytes,0,bytes.length,0);
      if(count>128)throw new Error('Token file is too large.');
      const token=bytes.subarray(0,count).toString('utf8').trim();
      if(!/^[a-f0-9]{64}$/.test(token))throw new Error('Token must be 32-byte hexadecimal text.');
      return token;
    } finally {bytes.fill(0);}
  } finally {closeSync(fd);}
}
export function invoiceArgs(body) {
  if (!body || Array.isArray(body) || typeof body !== 'object' ||
      Object.keys(body).some(k=>!['reference','amount_darks','description','expires_in'].includes(k)) ||
      !reference(body.reference)) throw new Error('Invalid invoice fields.');
  if (typeof body.amount_darks !== 'string' || !/^[1-9][0-9]{0,15}$/.test(body.amount_darks) || BigInt(body.amount_darks)>MAX_AMOUNT)
    throw new Error('amount_darks must be a positive integer string within the supply cap.');
  const description = body.description ?? '';
  if (typeof description !== 'string' || Buffer.byteLength(description)>64 || /[\x00-\x1f\x7f]/.test(description)) throw new Error('Invalid description.');
  const args=['invoice','--reference',body.reference,'--darks',body.amount_darks,'--description',description];
  if (body.expires_in !== undefined) {
    if (!Number.isSafeInteger(body.expires_in) || body.expires_in<1 || body.expires_in>604800) throw new Error('expires_in must be 1–604800 seconds.');
    args.push('--expires-in',String(body.expires_in));
  }
  return args;
}
export function cliBackend({binary,datadir,rpc,confirmations=10,viewKeyFile}) {
  if (!isAbsolute(binary) || !isAbsolute(datadir)) throw new Error('Use absolute binary and Devnet data paths.');
  const dir=realpathSync(datadir);
  if (readFileSync(join(dir,'.nightfall-checkout-devnet'),'utf8') !== 'devnet\n') throw new Error('Missing explicit Devnet opt-in marker.');
  if (!/^127\.0\.0\.1:[0-9]{1,5}$/.test(rpc) || Number(rpc.split(':')[1])<1 || Number(rpc.split(':')[1])>65535) throw new Error('RPC must use a literal loopback address.');
  if (!Number.isSafeInteger(confirmations) || confirmations<1 || confirmations>10000) throw new Error('Invalid confirmation policy.');
  const base=['--network','devnet','--datadir',dir,'--rpc',rpc];
  if(!viewKeyFile || !isAbsolute(viewKeyFile))throw new Error('Use an absolute owner-only view-key file and the nightfall-till binary.');
  base.push('--view-key-file',viewKeyFile);
  const run=async args => (await execute(binary,[...base,...args],{timeout:120000,maxBuffer:1024*1024,encoding:'utf8',env:process.env})).stdout;
  return {
    async create(body) {
      const args=invoiceArgs(body);
      if(existsSync(join(dir,'invoices',body.reference+'.json')))throw Object.assign(new Error('Invoice reference already exists.'),{status:409});
      const request=(await run(args)).trim();
      if (!request.startsWith('nightfall:') || !request.includes('?network=devnet&') || request.length>2048) throw new Error('Invalid CLI request.');
      return {reference:body.reference,request,network:'devnet',payment_observed:false};
    },
    async status(id) {
      const lines=(await run(['till','--confirmations',String(confirmations),'--json'])).trim();
      const row=lines.split('\n').filter(Boolean).map(line=>JSON.parse(line)).find(row=>row.reference===id);
      if (!row) return null;
      if (!/^(not_paid|unconfirmed|confirmed|closed|confirming:[0-9]+\/[0-9]+)$/.test(row.settlement)) throw new Error('Unrecognized CLI settlement.');
      return {...row,network:'devnet',confirmation_policy:confirmations,reorg_risk:true,observed_at:new Date().toISOString()};
    }
  };
}
export function createCheckoutServer({token,backend}) {
  if (typeof token!=='string' || !/^[a-f0-9]{64}$/.test(token)) throw new Error('Use a random 32-byte hexadecimal bearer token.');
  const expected=digest(`Bearer ${token}`);
  let tail=Promise.resolve(), queued=0;
  const serialize=async operation=>{
    if (queued>=8) throw Object.assign(new Error('Busy'),{status:429});
    queued++;
    const job=tail.then(operation); tail=job.catch(()=>{});
    try{return await job;}finally{queued--;}
  };
  const server=http.createServer(async(req,res)=>{
    const reply=(code,value)=>{res.writeHead(code,{'Content-Type':'application/json','Cache-Control':'no-store','X-Content-Type-Options':'nosniff'});res.end(JSON.stringify(value));};
    // No browser clients or DNS aliases. Loopback binding alone is insufficient
    // protection against cross-origin requests to localhost.
    if((req.headers.origin && req.headers.origin!==`http://127.0.0.1:${server.address().port}`) || req.headers.host!==`127.0.0.1:${server.address().port}`) return reply(403,{error:'Forbidden origin or host.'});
    if(req.method==='GET' && ['/', '/checkout.css', '/checkout.js'].includes(req.url)) {
      const asset=req.url==='/'?'index.html':req.url.slice(1);
      const type=asset.endsWith('.html')?'text/html':asset.endsWith('.css')?'text/css':'text/javascript';
      res.writeHead(200,{'Content-Type':type,'Cache-Control':'no-store','Content-Security-Policy':"default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",'X-Content-Type-Options':'nosniff'});res.end(readFileSync(join(uiRoot,asset)));return;
    }
    if(!timingSafeEqual(digest(req.headers.authorization || ''),expected)) return reply(401,{error:'Authentication required.'});
    if(req.method==='GET' && req.url==='/health') return reply(200,{network:'devnet',view_only:true});
    if(req.method==='POST' && req.url==='/invoices') {
      if(req.headers['content-type']!=='application/json') return reply(415,{error:'Use application/json.'});
      let data=Buffer.alloc(0);
      try {
        for await(const chunk of req) {data=Buffer.concat([data,chunk]);if(data.length>4096) return reply(413,{error:'Request too large.'});}
        let body;
        try {body=JSON.parse(data.toString('utf8'));invoiceArgs(body);} catch {return reply(400,{error:'Invalid invoice fields.'});}
        const result=await serialize(()=>backend.create(body));reply(201,result);
      } catch(error) {if(error.status===409)reply(409,{error:'This invoice reference already exists. Use a new reference or check the existing invoice.'});else reply(error.status===429?429:503,{error:'Invoice result unavailable. Check this reference before retrying; it may already exist.'});}
      return;
    }
    const match=req.url?.match(/^\/invoices\/([A-Za-z0-9_-]{1,48})$/);
    if(req.method==='GET' && match) {
      try {const row=await serialize(()=>backend.status(match[1]));reply(row?200:404,row || {error:'Unknown invoice.'});}
      catch(error){reply(error.status===429?429:503,{error:'Fresh chain status unavailable. Do not rely on an earlier result.'});}
      return;
    }
    reply(404,{error:'Unknown route.'});
  });
  server.requestTimeout=10000;server.headersTimeout=10000;server.keepAliveTimeout=1000;
  return server;
}
if (process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try {
    const args=process.argv.slice(2), config={};
    for(let i=0;i<args.length;i+=2){if(!['--wallet-bin','--datadir','--rpc','--token-file','--port','--confirmations','--view-key-file'].includes(args[i])||!args[i+1]||config[args[i]])throw new Error('Expected named option/value pairs.');config[args[i]]=args[i+1];}
    const token=readTokenFile(config['--token-file']);
    if(!config['--view-key-file'])throw new Error('Provide --view-key-file and the nightfall-till binary; spending wallets are not supported by the service CLI.');
    const backend=cliBackend({viewKeyFile:config['--view-key-file'],binary:config['--wallet-bin'],datadir:config['--datadir'],rpc:config['--rpc'],confirmations:Number(config['--confirmations']||10)});
    const port=Number(config['--port']||4187);if(!Number.isInteger(port)||port<1024||port>65535)throw new Error('Invalid local port.');
    const server=createCheckoutServer({token,backend});
    server.on('error',()=>{console.error('Cannot start local checkout listener.');process.exitCode=1;});
    server.listen(port,'127.0.0.1',()=>console.log(`Devnet-only checkout on 127.0.0.1:${port}; no payment submission routes.`));
    for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>server.close());
  }catch(error){console.error(error.message);process.exitCode=1;}
}
