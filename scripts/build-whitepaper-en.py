#!/usr/bin/env python3
"""One editorial source -> branded PDF and accessible static HTML.

Run with the bundled Python runtime. --publish copies a verified PDF to the
requested root filename and the website's versioned download path. No network,
Git operations or deployment are performed by this builder.
"""
from __future__ import annotations
import argparse
import hashlib
import html
import json
import math
import re
import shutil
from pathlib import Path
from decimal import Decimal
from reportlab.lib.colors import HexColor
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas
from reportlab.platypus import Paragraph, Table, TableStyle

ROOT = Path(__file__).resolve().parent.parent
PUBLIC = ROOT / 'website/public'
DATA = json.loads((ROOT / 'docs/whitepaper-en.json').read_text())
OUT = ROOT / 'output/pdf/NIGHTFALLCOIN-Whitepaper-EN.pdf'
PDF_NAME = 'NIGHTFALLCOIN-Whitepaper-EN-2026-09-18.pdf'
COLORS = {k: HexColor(v) for k,v in {
    'bg':'#423858','deep':'#332c4d','surface':'#302747','surface2':'#4b4064',
    'border':'#665777','text':'#f6f2ff','dim':'#d4c9e4','faint':'#c8bcdb',
    'violet':'#bda2ff','pink':'#e79cef','cyan':'#78dbec','ink':'#251c3a','warn':'#ffc85c',
}.items()}
W,H = A4
LEFT,RIGHT = 48,48
CW = W-LEFT-RIGHT
REFNUM = {key:i+1 for i,key in enumerate(DATA['references'])}
QA = []

def esc(text): return html.escape(str(text), quote=True)
def units(darks):
    s=f'{Decimal(darks)/Decimal(100000000):,.8f}'
    return s.rstrip('0').rstrip('.')

def schedule():
    rows=[]
    total=0
    for era in range(30):
        reward=600000000 >> era
        total+=reward*7500000
        rows.append([str(era),f'{era*7500000:,}',units(reward),units(total)])
    assert total==8999999925000000
    return rows

def register_fonts():
    for face,name in [('Regular','Inter'),('Medium','InterMedium'),('SemiBold','InterSemi'),('Bold','InterBold')]:
        pdfmetrics.registerFont(TTFont(name,str(ROOT/f'scripts/whitepaper-fonts/Inter-{face}.ttf')))
    pdfmetrics.registerFontFamily('Inter',normal='Inter',bold='InterBold',italic='Inter',boldItalic='InterBold')

def para(text,size=10.15,leading=15.25,color='text',font='Inter',raw=False):
    return Paragraph(text if raw else esc(text),ParagraphStyle('p',fontName=font,fontSize=size,
        leading=leading,textColor=COLORS[color],splitLongWords=True,allowWidows=0,allowOrphans=0))

def draw_p(c,text,x,y,width=CW,size=10.15,leading=15.25,color='text',font='Inter',raw=False):
    p=para(text,size,leading,color,font,raw)
    _,height=p.wrap(width,2000)
    p.drawOn(c,x,y-height)
    return y-height

def tracked(c,text,x,y,size=8,color='faint',spacing=1.25):
    c.saveState()
    obj=c.beginText(x,y)
    obj.setFont('InterSemi',size);obj.setCharSpace(spacing);obj.setFillColor(COLORS[color]);obj.textOut(text)
    c.drawText(obj)
    c.restoreState()

def roundbox(c,x,y,w,h,color='surface',radius=16,stroke=True):
    c.setFillColor(COLORS[color]);c.setStrokeColor(COLORS['border']);c.setLineWidth(.55)
    c.roundRect(x,y,w,h,radius,fill=1,stroke=int(stroke))

def gradient(c,x,y,w,h,radius=18):
    c.saveState()
    path=c.beginPath();path.roundRect(x,y,w,h,radius);c.clipPath(path,stroke=0)
    stops=[COLORS['violet'],COLORS['pink'],COLORS['cyan']]
    for i in range(250):
        t=i/249*2;k=min(int(t),1);f=t-k;a,b=stops[k],stops[k+1]
        c.setFillColorRGB(a.red*(1-f)+b.red*f,a.green*(1-f)+b.green*f,a.blue*(1-f)+b.blue*f)
        c.rect(x+w*i/250,y,w/250+1,h,fill=1,stroke=0)
    c.restoreState()

def background(c,page,title='WHITEPAPER'):
    c.setFillColor(COLORS['bg']);c.rect(0,0,W,H,fill=1,stroke=0)
    if page>1:
        roundbox(c,32,H-61,W-64,33,'deep',14)
        c.drawImage(str(ROOT/'assets/logo/mark-white-128.png'),45,H-53,17,17,mask='auto')
        tracked(c,'NIGHTFALL',70,H-47,8,'text',1.15)
        c.setFont('Inter',7.5);c.setFillColor(COLORS['faint']);c.drawRightString(W-46,H-47,'WHITEPAPER / 08 SEP 2026')
    c.setStrokeColor(COLORS['border']);c.setLineWidth(.5);c.line(LEFT,42,W-RIGHT,42)
    c.setFont('Inter',7.3);c.setFillColor(COLORS['faint']);c.drawString(LEFT,27,'NIGHTFALLCOIN / Protocol 8 / Software 1.0.4')
    c.drawRightString(W-RIGHT,27,f'{page:02d} / {len(DATA["sections"])+2:02d}')
    c.linkURL('https://nightfallcoin.org/whitepaper/',(LEFT,22,LEFT+220,37),relative=0,thickness=0)

def cover(c):
    background(c,1)
    c.bookmarkPage('cover');c.addOutlineEntry('NIGHTFALL Whitepaper','cover',0)
    c.drawImage(str(ROOT/'assets/logo/mark-white-256.png'),LEFT,H-120,42,42,mask='auto')
    tracked(c,'NIGHTFALLCOIN',LEFT+56,H-102,11,'text',2)
    tracked(c,'ENGLISH EDITION / SEPTEMBER 2026',LEFT,H-169,8.5,'cyan',1.5)
    y=draw_p(c,'Private settlement.',LEFT,H-204,size=35,leading=42,font='InterBold')
    y=draw_p(c,'Verifiable supply.',LEFT,y-3,size=35,leading=42,font='InterBold',color='violet')
    y=draw_p(c,'A technical whitepaper on confidential payments, monetary accounting and the limits of the current implementation.',LEFT,y-25,width=420,size=12,leading=19,color='dim')
    # Abstract contour motif, matching the site's supply card; no raster artwork.
    gradient(c,LEFT,242,CW,142,24)
    c.saveState();p=c.beginPath();p.roundRect(LEFT,242,CW,142,24);c.clipPath(p,stroke=0)
    c.setStrokeColor(COLORS['ink']);c.setStrokeAlpha(.12);c.setLineWidth(.85)
    for i in range(18):
        path=c.beginPath()
        for j in range(101):
            x=LEFT+j*CW/100;y=258+i*5+24*math.sin(j/18+i*.055)
            if j==0:path.moveTo(x,y)
            else:path.lineTo(x,y)
        c.drawPath(path)
    c.restoreState()
    tracked(c,'WHITEPAPER',LEFT+24,353,9,'ink',2.4)
    draw_p(c,'A native privacy currency.',LEFT+24,330,CW-48,21,27,'ink','InterBold')
    draw_p(c,'Confidential values. One-time outputs. Explicit monetary rules.',LEFT+24,293,CW-48,10,15,'ink')
    for i,(n,label) in enumerate([('0','Premine'),('90M','Supply ceiling'),('15s','Target block time')]):
        x=LEFT+i*(CW+12)/3;cw=(CW-24)/3
        roundbox(c,x,140,cw,75)
        draw_p(c,n,x+16,198,cw-32,24,27,'text','InterBold')
        draw_p(c,label,x+16,163,cw-32,8.5,12,'dim')
    draw_p(c,'Release 1.0.4 / Protocol 8 / Wire 6',LEFT,113,size=9,leading=13,color='dim')
    draw_p(c,'Independent review outstanding. Mainnet swaps disabled.',LEFT,89,size=8.8,leading=13,color='faint')
    c.showPage()

def contents(c):
    background(c,2);c.bookmarkPage('contents');c.addOutlineEntry('Contents','contents',0)
    tracked(c,'A READING MAP',LEFT,H-95,8,'cyan')
    y=draw_p(c,'The design. The evidence.\nThe limits.'.replace('\n',' '),LEFT,H-116,size=26,leading=32,font='InterBold')
    y=draw_p(c,'Start with the thesis and trust boundaries. Use the later chapters for implementation detail, risk analysis and source references.',LEFT,y-18,size=11,leading=17,color='dim')-25
    for i,s in enumerate(DATA['sections']):
        top=y-i*23.2
        c.setFont('InterSemi',9);c.setFillColor(COLORS['violet']);c.drawString(LEFT,top,f'{i+1:02d}')
        c.setFont('Inter',9.5);c.setFillColor(COLORS['text']);c.drawString(LEFT+30,top,s['title'])
        c.setFont('Inter',8);c.setFillColor(COLORS['faint']);c.drawRightString(W-RIGHT,top,f'{i+3:02d}')
        c.linkRect('',s['id'],(LEFT,top-5,W-RIGHT,top+12),relative=0,thickness=0)
    draw_p(c,'This document separates implemented behavior, known limitations and proposed research. It does not announce an investment product or certify security.',LEFT,106,size=9,leading=14,color='dim')
    c.showPage()

def table(c,headers,rows,y,compact=False):
    n=len(headers)
    widths={2:[.37,.63],3:[.23,.35,.42],4:[.10,.25,.23,.42]}[n]
    size,leading,pad=(8.1,10.4,2.4) if compact else (8.8,12.4,5)
    values=[[para(v,size,leading,'violet','InterSemi') for v in headers]]+[[para(v,size,leading,'text') for v in row] for row in rows]
    t=Table(values,colWidths=[CW*f for f in widths])
    t.setStyle(TableStyle([
        ('BACKGROUND',(0,0),(-1,0),COLORS['deep']),('ROWBACKGROUNDS',(0,1),(-1,-1),[COLORS['surface'],COLORS['surface2']]),
        ('VALIGN',(0,0),(-1,-1),'TOP'),('LEFTPADDING',(0,0),(-1,-1),9),('RIGHTPADDING',(0,0),(-1,-1),9),
        ('TOPPADDING',(0,0),(-1,-1),pad),('BOTTOMPADDING',(0,0),(-1,-1),pad),
        ('LINEBELOW',(0,0),(-1,0),.6,COLORS['border']),
    ]))
    _,height=t.wrap(CW,2000);t.drawOn(c,LEFT,y-height)
    return y-height-16

def block(c,b,y):
    typ=b['type']
    if typ=='p':return draw_p(c,b['text'],LEFT,y)-12
    if typ=='h':return draw_p(c,b['text'],LEFT,y-2,size=13,leading=17,font='InterSemi')-9
    if typ=='table':return table(c,b['headers'],b['rows'],y)
    if typ=='schedule':return table(c,['Era','First block','NIGHT / block','Cumulative NIGHT'],schedule(),y,True)
    if typ=='callout':
        p=para(b['text'],9.3,13.6,'dim');_,h=p.wrap(CW-32,2000)
        title=para(b['title'],10,14,'violet','InterSemi');_,th=title.wrap(CW-32,2000)
        height=h+th+36
        roundbox(c,LEFT,y-height,CW,height,'deep',14)
        c.setStrokeColor(COLORS['violet']);c.setLineWidth(2);c.line(LEFT+1,y-height+14,LEFT+1,y-14)
        title.drawOn(c,LEFT+16,y-16-th);p.drawOn(c,LEFT+16,y-22-th-h)
        return y-height-14
    if typ=='formula':
        line_size=10.2
        for line in b['lines']:
            assert pdfmetrics.stringWidth(line,'InterMedium',line_size)<CW-34,line
        height=len(b['lines'])*18+24
        roundbox(c,LEFT,y-height,CW,height,'surface',13)
        for i,line in enumerate(b['lines']):draw_p(c,line,LEFT+16,y-14-i*18,CW-32,line_size,14,'cyan','InterMedium')
        y=draw_p(c,b['caption'],LEFT,y-height-8,size=8.4,leading=12,color='dim')
        return y-15
    if typ in ('cards','metrics','flow'):
        if typ=='cards':
            if b.get('columns') == 3:
                cw=(CW-20)/3
                cells=[]
                for title,text in b['items']:
                    p=para(text,9.3,13.5,'dim');_,ph=p.wrap(cw-26,2000)
                    t=para(title,10,14,'violet','InterSemi');_,th=t.wrap(cw-26,2000)
                    cells.append((p,ph,t,th))
                height=max(ph+th+36 for p,ph,t,th in cells)
                for i,(p,ph,t,th) in enumerate(cells):
                    x=LEFT+i*(cw+10);roundbox(c,x,y-height,cw,height)
                    t.drawOn(c,x+13,y-13-th);p.drawOn(c,x+13,y-22-th-ph)
                return y-height-18
            for title,text in b['items']:
                p=para(text,9.3,13.5,'dim');_,h=p.wrap(CW-32,2000)
                height=h+43
                roundbox(c,LEFT,y-height,CW,height)
                draw_p(c,title,LEFT+16,y-12,CW-32,10,14,'violet','InterSemi');p.drawOn(c,LEFT+16,y-height+12)
                y-=height+9
            return y-5
        count=len(b['items']);gap=10;cw=(CW-gap*(count-1))/count
        if typ=='metrics':
            for i,(number,title) in enumerate(b['items']):
                x=LEFT+i*(cw+gap);roundbox(c,x,y-75,cw,75)
                draw_p(c,number,x+13,y-12,cw-26,25,29,'violet','InterBold')
                draw_p(c,title,x+13,y-48,cw-26,8.4,11.5,'dim')
            return y-93
        height=102
        for i,(title,text) in enumerate(b['items']):
            x=LEFT+i*(cw+gap);roundbox(c,x,y-height,cw,height)
            draw_p(c,f'{i+1:02d}',x+12,y-10,cw-24,8,11,'cyan','InterSemi')
            py=draw_p(c,title,x+12,y-29,cw-24,9.3,12.4,'text','InterSemi')
            py=draw_p(c,text,x+12,py-5,cw-24,8.5,11.6,'dim')
            assert py>=y-height+5,(title,py,y-height)
        return y-height-18
    if typ=='chart':
        height=184;roundbox(c,LEFT,y-height,CW,height)
        draw_p(c,b['title'],LEFT+16,y-13,CW-32,10,14,'text','InterSemi')
        x0=LEFT+45;bottom=y-142;plotw=CW-75;ploth=88
        c.setFont('Inter',7);c.setLineWidth(.4)
        for pct in [0,50,100]:
            yy=bottom+ploth*pct/100;c.setStrokeColor(COLORS['border']);c.line(x0,yy,x0+plotw,yy)
            c.setFillColor(COLORS['faint']);c.drawRightString(x0-8,yy-2,str(pct)+'%')
        c.setStrokeColor(COLORS['cyan']);c.setLineWidth(2)
        path=c.beginPath();path.moveTo(x0,bottom)
        for era in range(10):
            pct=sum((600000000>>e)*7500000 for e in range(era+1))/9000000000000000
            path.lineTo(x0+(era+1)*plotw/10,bottom+ploth*pct)
        c.drawPath(path)
        c.setFillColor(COLORS['faint']);c.setFont('Inter',7)
        for e in [0,1,2,4,9]:c.drawCentredString(x0+(e+1)*plotw/10,bottom-13,str(e))
        c.drawRightString(x0+plotw,bottom-28,'End of era (first 10 shown)')
        return draw_p(c,b['caption'],LEFT,y-height-8,size=8.4,leading=12,color='dim')-14
    if typ=='references':
        for key,r in DATA['references'].items():
            title=f'<font color="#bda2ff">[{REFNUM[key]:02d}]</font> <link href="{esc(r["url"])}" color="#f6f2ff">{esc(r["title"])}</link>'
            y=draw_p(c,title,LEFT,y,size=8.5,leading=12,raw=True)-8
        return y-4
    raise ValueError(typ)

def build_pdf():
    register_fonts();OUT.parent.mkdir(parents=True,exist_ok=True)
    c=canvas.Canvas(str(OUT),pagesize=A4,pageCompression=1,invariant=1)
    c.setTitle('NIGHTFALL - Private settlement. Verifiable supply.');c.setAuthor('NIGHTFALLCOIN')
    c.setSubject('English whitepaper / 18 September 2026 / release 1.0.4 / protocol 8')
    c.setKeywords('Nightfall, NIGHT, whitepaper, confidential transactions, proof of work, privacy')
    c.setViewerPreference('DisplayDocTitle','true')
    cover(c);contents(c)
    for index,s in enumerate(DATA['sections']):
        page=index+3;background(c,page);c.bookmarkPage(s['id']);c.addOutlineEntry(s['title'],s['id'],0)
        tracked(c,s['label'].upper(),LEFT,H-93,8,'cyan')
        y=draw_p(c,s['title'],LEFT,H-111,size=24.5,leading=30,font='InterBold')
        y=draw_p(c,s['lead'],LEFT,y-13,size=11.2,leading=16.5,color='dim')-21
        for b in s['blocks']:
            y=block(c,b,y)
        QA.append({'page':page,'chapter':s['id'],'bottom':round(y,2)})
        refs=' / '.join(f'<link href="{esc(DATA["references"][k]["url"])}" color="#c8bcdb">[{REFNUM[k]:02d}]</link>' for k in s['sources'])
        if refs:draw_p(c,'Sources '+refs,LEFT,59,size=7,leading=9,color='faint',raw=True)
        c.showPage()
    overflows=[r for r in QA if r['bottom']<64]
    if overflows:raise RuntimeError(f'Pages overflow; edit layout/content, never clip: {overflows}')
    c.save()
    (ROOT/'tmp/pdfs').mkdir(parents=True,exist_ok=True)
    (ROOT/'tmp/pdfs/whitepaper-layout.json').write_text(json.dumps(QA,indent=2))

def html_table(headers,rows,caption=None):
    head=''.join(f'<th scope="col">{esc(x)}</th>' for x in headers)
    body=''.join('<tr>'+''.join(f'<td>{esc(x)}</td>' for x in row)+'</tr>' for row in rows)
    cap=f'<caption>{esc(caption)}</caption>' if caption else ''
    return f'<div class="wp-table" tabindex="0" role="region" aria-label="Scrollable reference table"><table>{cap}<thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>'

def html_block(b):
    typ=b['type']
    if typ=='p':return f'<p>{esc(b["text"])}</p>'
    if typ=='h':return f'<h3>{esc(b["text"])}</h3>'
    if typ=='callout':return f'<aside class="wp-callout"><h3>{esc(b["title"])}</h3><p>{esc(b["text"])}</p></aside>'
    if typ=='formula':return '<figure class="wp-formula"><div>'+''.join(f'<span>{esc(line)}</span>' for line in b['lines'])+f'</div><figcaption>{esc(b["caption"])}</figcaption></figure>'
    if typ=='table':return html_table(b['headers'],b['rows'])
    if typ=='schedule':return html_table(['Era','First block','NIGHT / block','Cumulative NIGHT'],schedule(),'Exact integer-halving schedule: 30 paying eras')
    if typ in ('cards','flow','metrics'):
        return f'<div class="wp-{typ}">'+''.join(f'<div><span class="wp-seq">{i+1:02d}</span><h3>{esc(title)}</h3><p>{esc(text)}</p></div>' for i,(title,text) in enumerate(b['items']))+'</div>'
    if typ=='chart':
        points=['50,160']
        for e in range(10):
            pct=sum((600000000>>i)*7500000 for i in range(e+1))/9000000000000000
            points.append(f'{50+(e+1)*47},{160-pct*112:.3f}')
        grid=''.join(f'<path d="M50 {160-p*1.12}H520"/><text x="40" y="{164-p*1.12}" text-anchor="end">{p}%</text>' for p in [0,50,100])
        labels=''.join(f'<text x="{50+(e+1)*47}" y="181" text-anchor="middle">{e}</text>' for e in [0,1,2,4,9])
        return f'<figure class="wp-chart"><h3>{esc(b["title"])}</h3><svg viewBox="0 0 560 210" role="img" aria-label="Scheduled cumulative issuance approaches the 90 million ceiling: 50 percent by era zero, 75 by era one, 87.5 by era two."><g class="wp-grid">{grid}</g><polyline points="{" ".join(points)}"/>{labels}<text x="520" y="202" text-anchor="end">End of era (first 10 shown)</text></svg><figcaption>{esc(b["caption"])}</figcaption></figure>'
    if typ=='references':return '<ol class="wp-references">'+''.join(f'<li id="ref-{key}"><a href="{esc(r["url"])}">{esc(r["title"])}</a></li>' for key,r in DATA['references'].items())+'</ol>'
    raise ValueError(typ)

def build_html():
    nav=re.search(r'<nav id="nav">[\s\S]*?</nav>',(PUBLIC/'build/index.html').read_text()).group()
    sections=''
    for s in DATA['sections']:
        refs=' '.join(f'<a href="#ref-{key}" aria-label="Source {REFNUM[key]}: {esc(DATA["references"][key]["title"])}">[{REFNUM[key]}]</a>' for key in s['sources'])
        sections+=f'<section class="wp-chapter" id="{s["id"]}"><p class="wp-label">{esc(s["label"])}</p><h2>{esc(s["title"])}</h2><p class="wp-lead">{esc(s["lead"])}</p>'+''.join(html_block(b) for b in s['blocks'])+(f'<p class="wp-sources">Sources {refs}</p>' if refs else '')+'</section>'
    toc=''.join(f'<li><a href="#{s["id"]}"><span>{i+1:02d}</span>{esc(s["title"])}</a></li>' for i,s in enumerate(DATA['sections']))
    finger=lambda p:hashlib.sha256((PUBLIC/p).read_bytes()).hexdigest()[:8]
    html_doc=f'''<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>NIGHTFALL Whitepaper - Private settlement. Verifiable supply.</title>
<meta name="description" content="The Nightfall 1.0 whitepaper: confidential payments, verifiable supply, proof of work, wallet trust, and an evidence-led research agenda.">
<meta name="theme-color" content="#423858"><link rel="canonical" href="https://nightfallcoin.org/whitepaper/">
<link rel="icon" href="/assets/favicon-32.png" sizes="32x32" type="image/png">
<link rel="stylesheet" href="/css/style.css?v={finger('css/style.css')}"><link rel="stylesheet" href="/css/whitepaper.css?v={finger('css/whitepaper.css')}">
</head><body><a class="skip-link" href="#content">Skip to whitepaper</a>{nav}
<main id="content" class="wp" tabindex="-1"><header class="wp-hero wrap"><div class="wp-hero-copy"><p class="wp-label">Whitepaper / English edition / 18 September 2026</p>
<h1>Private settlement.<br><span class="grad-text">Verifiable supply.</span></h1>
<p class="wp-intro">The design, the evidence and the limits of NIGHTFALLCOIN.</p>
<div class="wp-actions"><a class="btn btn-primary" href="#abstract">Read the whitepaper</a><a class="btn btn-outline" href="/downloads/{PDF_NAME}">Download PDF</a></div>\n<p class="wp-prev">Previous edition: <a href="/downloads/NIGHTFALLCOIN-Whitepaper-EN-2026-09-08.pdf">8 September 2026 (release 0.9.5)</a> — kept as published. Its chapter on atomic swaps describes a feature that has since been withdrawn.</p>
<p class="wp-edition">Software 1.0.4 <span>Protocol 8</span><span>20 chapters</span></p></div>
<div class="wp-cover" aria-label="Nightfall whitepaper cover"><img src="/assets/logo-256.png" width="64" height="64" alt=""><p>NIGHTFALL</p><strong>Whitepaper</strong><span>Confidential values.<br>Explicit monetary rules.</span><small>English / September 2026</small></div></header>
<div class="wrap wp-layout"><aside class="wp-toc"><details open><summary>In this paper <span>20 chapters</span></summary><ol>{toc}</ol></details></aside>
<article class="wp-article" aria-label="Whitepaper chapters">{sections}<div class="wp-end"><a class="btn btn-primary" href="/downloads/{PDF_NAME}">Download the PDF edition</a><a href="#content">Back to the beginning</a></div></article></div></main>
<footer><div class="wrap"><a class="brand" href="/">NIGHTFALL</a><p>English whitepaper / 18 September 2026. Same editorial source as the PDF.</p><a href="/audit/">Current security review</a> · <a href="/#download">Wallet downloads</a></div></footer>
<script src="/js/nav.js?v={finger('js/nav.js')}"></script><script src="/js/whitepaper.js?v={finger('js/whitepaper.js')}"></script></body></html>'''
    (PUBLIC/'whitepaper').mkdir(parents=True,exist_ok=True)
    (PUBLIC/'whitepaper/index.html').write_text(html_doc)

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--publish',action='store_true');ap.add_argument('--html-only',action='store_true');args=ap.parse_args()
    if not args.html_only:build_pdf()
    build_html()
    if args.publish:
        if args.html_only:raise ValueError('--publish requires a fresh PDF build')
        shutil.copyfile(OUT,ROOT/'NIGHTFALLCOIN-Whitepaper-EN.pdf')
        shutil.copyfile(OUT,PUBLIC/'downloads'/PDF_NAME)
    print(f'Whitepaper: {len(DATA["sections"])} chapters; PDF {OUT}; HTML /whitepaper/; published copies={args.publish}')
    for row in QA:print(row)

if __name__=='__main__':main()
