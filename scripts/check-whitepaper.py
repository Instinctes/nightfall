#!/usr/bin/env python3
"""Regression checks for the shared English PDF / HTML edition (no network)."""
import hashlib
import importlib.util
import json
import re
import subprocess
from html.parser import HTMLParser
from pathlib import Path
from pypdf import PdfReader
import pdfplumber

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('whitepaper', ROOT/'scripts/build-whitepaper-en.py')
paper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paper)
data = paper.DATA
normalize = lambda text: ' '.join(text.split())

class Text(HTMLParser):
    def __init__(self):
        super().__init__(); self.parts = []
    def handle_data(self, value): self.parts.append(value)

html = (paper.PUBLIC/'whitepaper/index.html').read_text()
parser = Text(); parser.feed(html)
webtext = normalize(' '.join(parser.parts))
reader = PdfReader(paper.OUT)
assert len(reader.pages) == len(data['sections'])+2 == 22
assert len(reader.outline) == 22
assert not reader.is_encrypted
assert '/JavaScript' not in str(reader.trailer)
assert 'NIGHTFALL' in reader.metadata.title
assert len(data['references']) == 16

def texts(block):
    for key in ['text','title','caption']:
        if key in block: yield block[key]
    yield from block.get('lines', [])
    yield from block.get('headers', [])
    for row in block.get('items', []) + block.get('rows', []): yield from row
    if block['type'] == 'schedule':
        for row in paper.schedule(): yield from row
    if block['type'] == 'references':
        for ref in data['references'].values(): yield ref['title']

checked = 0
for i, section in enumerate(data['sections']):
    pdftext = normalize(reader.pages[i+2].extract_text())
    expected = [section['title'], section['lead']]
    for block in section['blocks']: expected.extend(texts(block))
    for text in expected:
        if not text: continue
        assert normalize(text) in pdftext, f"Missing PDF text: {section['id']}: {text}"
        assert normalize(text) in webtext, f"Missing HTML text: {section['id']}: {text}"
        checked += 1
    for key in section['sources']: assert key in data['references']

with pdfplumber.open(paper.OUT) as pdf:
    for i,page in enumerate(pdf.pages):
        for char in page.chars:
            assert char['x0'] >= 30 and char['x1'] <= page.width-30, (i+1, 'horizontal clipping', char['text'])
            assert char['top'] >= 20 and char['bottom'] <= page.height-20, (i+1, 'vertical clipping', char['text'])
            assert char['text'] != '\ufffd', (i+1, 'replacement glyph')
for row in json.loads((ROOT/'tmp/pdfs/whitepaper-layout.json').read_text()):
    assert row['bottom'] >= 64, row

source = (ROOT/'crates/nightfall-types/src/lib.rs').read_text()
assert re.search(r'^version\s*=\s*"'+re.escape(data['version'])+'"', (ROOT/'Cargo.toml').read_text(), re.M)
for key,value in [('PROTOCOL_VERSION',data['protocol']),('WIRE_VERSION',data['wire']),
                  ('DARKS_PER_NIGHT',100000000),('MAX_SUPPLY_NIGHT',90000000),
                  ('INITIAL_BLOCK_REWARD_NIGHT',6),('HALVING_INTERVAL_BLOCKS',7500000),
                  ('TARGET_BLOCK_TIME_SECS',15)]:
    actual = re.search(r'pub const '+key+r':\s*u\d+\s*=\s*([\d_]+)',source)
    assert actual and int(actual[1].replace('_','')) == value, key
manifest = json.loads((paper.PUBLIC/'chain/bootstrap.json').read_text())
assert data['genesis'] == manifest['genesis']
assert ''.join(data['sections'][18]['blocks'][1]['lines']) == data['genesis']
assert len(data['genesis']) == 64
assert paper.schedule()[-1] == ['29','217,500,000','0.00000001','89,999,999.25']
assert (600000000 >> 30) == 0
tag = 'v'+data['version']
commit = subprocess.check_output(['git','-C',str(ROOT/'github'),'rev-parse',tag],text=True).strip()
assert commit == data['commit']
for ref in data['references'].values():
    assert ref['url'].startswith('https://')
    if 'github.com/Instinctes/nightfall/' in ref['url']:
        path = ref['url'].split('/'+tag+'/')[1]
        subprocess.run(['git','-C',str(ROOT/'github'),'cat-file','-e',tag+':'+path],check=True)
assert not any(c in json.dumps(data,ensure_ascii=False) for c in '\u2010\u2011\u2012\u2013\u2014')
assert '<h1>' in html and html.count('class="wp-chapter"') == 20
assert not re.search(r'<script(?![^>]*\bsrc=)',html)
assert 'href="/whitepaper/"' in (paper.PUBLIC/'index.html').read_text()
for asset in ['css/style.css','css/whitepaper.css','js/nav.js','js/whitepaper.js']:
    fingerprint = hashlib.sha256((paper.PUBLIC/asset).read_bytes()).hexdigest()[:8]
    assert f'{asset}?v={fingerprint}' in html, asset
if '--published' in __import__('sys').argv:
    expected = paper.OUT.read_bytes()
    assert expected == (ROOT/'NIGHTFALLCOIN-Whitepaper-EN.pdf').read_bytes()
    assert expected == (paper.PUBLIC/'downloads'/paper.PDF_NAME).read_bytes()
print(f'whitepaper ok: 22 pages, 20 chapters, {checked} shared text fragments, 30 integer eras, 16 references, source baseline, bounds and links')
