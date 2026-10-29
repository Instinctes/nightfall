import assert from 'node:assert/strict';
import { paymentLink, paymentFragment, consumePaymentFragment, SHARE_PREFIX } from '../website/public/wallet-origin/pay-link.js';
// Public dummy transport text, deliberately not an address. Rust validates it.
const uri = 'nightfall:nf1PUBLIC?network=devnet&amount=123456789&memo=Coffee%20%26%20tea&id=A%231';
const link = paymentLink(uri);
assert.equal(paymentFragment(new URL(link).hash), uri);
assert.equal(new URL(link).search, '');
assert.equal(new URL(link).pathname, '/');
assert.equal(paymentFragment('#settings'), null);
let replaced;
assert.equal(consumePaymentFragment({hash:new URL(link).hash,pathname:'/',search:''},{replaceState(...args){replaced=args;}}),uri);
assert.deepEqual(replaced,[null,'','/']);
for(const hash of ['#pay=%', '#pay=%FF', '#pay=nightfall:raw', '#pay='+encodeURIComponent(link), '#pay='+ 'x'.repeat(6200), '#pay='+encodeURIComponent(uri)+'&extra=1']) {
  replaced=null;
  assert.throws(()=>consumePaymentFragment({hash,pathname:'/',search:''},{replaceState(...args){replaced=args;}}));
  assert.deepEqual(replaced,[null,'','/']);
}
assert.equal(paymentLink("nightfall:nf1?network=devnet&memo=!'()*+"), SHARE_PREFIX+'nightfall%3Anf1%3Fnetwork%3Ddevnet%26memo%3D%21%27%28%29%2A%2B');
assert.throws(()=>paymentLink('nightfall:'+ '🌙'.repeat(600)));
console.log('Pay link transport: roundtrip, no query leakage, early removal and malformed-input rejection passed.');
