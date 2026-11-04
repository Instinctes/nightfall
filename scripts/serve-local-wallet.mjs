#!/usr/bin/env node
// Local review/self-hosting helper. Fixed static root and loopback node only.
import http from 'node:http';
import {readFileSync,existsSync,statSync,realpathSync} from 'node:fs';
import {resolve,extname} from 'node:path';
const root=realpathSync(resolve(process.argv[2]||'website/public/wallet-origin'));
const port=Number(process.argv[3]||4185), upstream=new URL(process.argv[4]||'http://127.0.0.1:17888/');
if(upstream.protocol!=='http:' || upstream.hostname!=='127.0.0.1' || upstream.username || upstream.password || upstream.search || upstream.hash || !Number.isInteger(port) || port<1024 || port>65535)throw new Error('Use a literal loopback upstream and a valid local port.');
const allowed=new Set(['status','wallet_scan','scan_feed','get_headers','submit_tx']);
const server=http.createServer(async(req,res)=>{
 res.setHeader('Cache-Control','no-store');res.setHeader('X-Content-Type-Options','nosniff');res.setHeader('Referrer-Policy','no-referrer');
 res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'");
 if(req.headers.host!==`127.0.0.1:${port}` || (req.headers.origin && req.headers.origin!==`http://127.0.0.1:${port}`)){res.writeHead(403).end();return;}
 if(req.url==='/wallet-api') {
  if(req.method!=='POST'){res.writeHead(405).end();return;}
  try{let body='';for await(const chunk of req){body+=chunk;if(Buffer.byteLength(body)>524288){res.writeHead(413).end();return;}}
   const call=JSON.parse(body);if(!allowed.has(call.method)){res.writeHead(403).end();return;}
   const reply=await fetch(upstream,{method:'POST',headers:{'Content-Type':'application/json'},body,redirect:'error',signal:AbortSignal.timeout(15000)});
   const chunks=[];let size=0;for await(const chunk of reply.body){size+=chunk.byteLength;if(size>16*1024*1024)throw new Error('Oversized response');chunks.push(Buffer.from(chunk));}res.writeHead(reply.status,{'Content-Type':'application/json'}).end(Buffer.concat(chunks));
  }catch{res.writeHead(503,{'Content-Type':'application/json'}).end(JSON.stringify({error:'Local node unavailable. Start an archive node with its light API on loopback.'}));}return;
 }
 if(req.method!=='GET'){res.writeHead(405).end();return;}
 let url;try{url=new URL(req.url,`http://127.0.0.1:${port}`);}catch{res.writeHead(400).end();return;}
 const path=resolve(root,'.'+url.pathname);const file=url.pathname==='/'?resolve(root,'index.html'):path;
 if(!file.startsWith(root+'/') || !existsSync(file) || !realpathSync(file).startsWith(root+'/') || !statSync(file).isFile()){res.writeHead(404).end();return;}
 res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.wasm':'application/wasm','.png':'image/png','.webmanifest':'application/manifest+json','.woff2':'font/woff2'})[extname(file)]||'application/octet-stream');res.end(readFileSync(file));
});
server.requestTimeout=20000;server.headersTimeout=10000;
server.listen(port,'127.0.0.1',()=>console.log(`Local wallet: http://127.0.0.1:${port}/ → ${upstream.origin}. Keys stay in this browser. Ctrl+C to stop.`));
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>server.close());
