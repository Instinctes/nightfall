#!/usr/bin/env node
// Site-only regressions: shared wallet palette, contrast and navigation.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const root = new URL('../', import.meta.url);
const read = path => readFileSync(new URL(path, root), 'utf8');
const css = read('website/public/css/style.css');
const wallet = read('website/public/wallet/style.css');
const token = (source, key) => {
    const value = source.match(new RegExp(`--${key}:\\s*(#[a-f0-9]{6})`, 'i'))?.[1];
    assert.ok(value, `missing ${key}`);
    return value.toLowerCase();
};
for (const key of ['bg', 'surface', 'ink']) {
    assert.equal(token(css, key), token(wallet, key), `wallet palette: ${key}`);
}
const rgb = hex => hex.slice(1).match(/../g).map(x => parseInt(x, 16));
const luminance = values => values.map(v => v / 255).map(v =>
    v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
).reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0);
const ratio = (a, b) => {
    const [lo, hi] = [luminance(a), luminance(b)].sort((x, y) => x - y);
    return (hi + 0.05) / (lo + 0.05);
};
for (const fg of ['text', 'text-dim', 'text-faint', 'violet-hi']) {
    for (const bg of ['bg', 'bg-2', 'surface', 'surface-2']) {
        assert.ok(ratio(rgb(token(css, fg)), rgb(token(css, bg))) >= 4.5,
            `${fg} on ${bg} must meet 4.5:1`);
    }
}
const ink = rgb(token(css, 'ink'));
for (const name of ['grad', 'grad-bright']) {
    const gradient = css.match(new RegExp(`--${name}:([^;]+)`))?.[1];
    const stops = gradient.match(/#[a-f0-9]{6}/gi).map(rgb);
    for (let segment = 1; segment < stops.length; segment++) {
        for (let i = 0; i <= 100; i++) {
            const color = stops[segment].map((v, c) =>
                v * i / 100 + stops[segment - 1][c] * (1 - i / 100));
            // Include the supply card's 8% ink contour overlay.
            assert.ok(ratio(ink, color.map((v, c) => v * 0.92 + ink[c] * 0.08)) >= 4.5,
                `${name} dark-label contrast at segment ${segment}, sample ${i}`);
        }
    }
}
assert.match(css, /\.btn-primary\s*\{[^}]*color:\s*var\(--ink\)/);
assert.match(css, /\[hidden\]\s*\{\s*display:\s*none\s*!important/);
assert.match(css, /:focus-visible\s*\{[^}]*outline:/);
assert.match(css, /max-width:\s*1000px/);
assert.doesNotMatch(css, /max-width:\s*820px/);
const nav = read('website/public/js/nav.js');
assert.match(nav, /min-width: 1001px/);
assert.match(nav, /aria-current/);
assert.match(nav, /drawer\.inert = !open/);
for (const page of ['index.html', 'chain/index.html', 'emission/index.html',
    'audit/index.html', 'audit/2026-08-16/index.html', 'build/index.html', 'view-key/index.html', 'whitepaper/index.html', 'compare/index.html', '404.html']) {
    const html = read(`website/public/${page}`);
    assert.match(html, /name="theme-color" content="#423858"/, page);
    assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)[^>]*>/, `${page}: CSP`);
    if (page !== '404.html') {
        assert.match(html, /class="skip-link"/, page);
        assert.match(html, /tabindex="-1"/, page);
    }
}
console.log('site design ok — wallet palette, text/gradient contrast, 10 pages, navigation');
