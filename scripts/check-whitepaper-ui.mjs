#!/usr/bin/env node
// Isolated interaction regression: no browser, network, wallet or storage access.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source=readFileSync(new URL('../website/public/js/whitepaper.js',import.meta.url),'utf8');
let cases=0;
for (const mobile of [false,true]) {
  for (const observerAvailable of [false,true]) {
    const toc={open:true};
    const links=['abstract','privacy','sources'].map(id=>({hash:'#'+id,attrs:{},
      setAttribute(k,v){this.attrs[k]=v;}, removeAttribute(k){delete this.attrs[k];},
      addEventListener(event,fn){assert.equal(event,'click');this.click=fn;}}));
    const chapters=links.map(link=>({id:link.hash.slice(1)}));
    const observed=[]; let callback;
    class Observer {constructor(fn){callback=fn;} observe(node){observed.push(node);}}
    const context={document:{querySelector:s=>{assert.equal(s,'.wp-toc details');return toc;},
      querySelectorAll:s=>s==='.wp-toc a'?links:chapters},matchMedia:()=>({matches:mobile}),
      window:observerAvailable?{IntersectionObserver:Observer}:{},IntersectionObserver:Observer};
    vm.runInNewContext(source,context);
    assert.equal(toc.open,!mobile);
    toc.open=true;links[1].click();
    assert.equal(links[1].attrs['aria-current'],'location');
    assert.equal(links.filter(l=>l.attrs['aria-current']).length,1);
    assert.equal(toc.open,!mobile);
    if(observerAvailable){
      assert.deepEqual(observed,chapters);
      callback([{isIntersecting:true,target:chapters[2],boundingClientRect:{top:150}}]);
      assert.equal(links[2].attrs['aria-current'],'location');
      assert.equal(links.filter(l=>l.attrs['aria-current']).length,1);
      callback([]);assert.equal(links[2].attrs['aria-current'],'location');
    }
    cases++;
  }
}
assert.doesNotMatch(source,/\b(?:fetch|localStorage|sessionStorage|eval)\s*[.(]/);
console.log(`whitepaper UI ok — ${cases} desktop/mobile and observer/fallback combinations; single active chapter, collapse and zero storage/network`);
