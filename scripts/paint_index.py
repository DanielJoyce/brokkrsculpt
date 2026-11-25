#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Read the miniature-paints tables and sort every paint into a kind.

The first pass of the classifier in docs/PAINT-RACK.md: the product line
decides first, then the paint's name, then a short list of exceptions.
It prints the count of each kind.

    scripts/paint_index.py <path to miniature-paints/paints> [out.json] [kind [brand]]

The tables are https://github.com/Arcturus5404/miniature-paints (MIT).
"""
import glob, re, collections, json, sys
SRC = sys.argv[1] if len(sys.argv) > 1 else 'miniature-paints/paints'
OUT = sys.argv[2] if len(sys.argv) > 2 else None
ARGS = sys.argv[3:]
rows=[]
BAD=[]
for f in sorted(glob.glob(SRC+'/*.md')):
    brand=f.split('/')[-1][:-3]
    lines=[l for l in open(f) if l.startswith('|')]
    head=[h.strip() for h in lines[0].strip().strip('|').split('|')]
    for l in lines[2:]:
        c=[x.strip() for x in l.strip().strip('|').split('|')]
        d=dict(zip(head,c))
        hexm=re.search(r'`(#[0-9A-Fa-f]{6})`',l)
        if not hexm: BAD.append(l); continue
        h=hexm.group(1).upper()
        rows.append(dict(brand=brand,name=d.get('Name',''),set=d.get('Set',''),code=d.get('Code',''),
                         rgb=(int(h[1:3],16),int(h[3:5],16),int(h[5:7],16)),hex=h,
                         shifted=not d.get('R','').isdigit()))
REFERENCE={'Pantone','RAL'}
# set-name rules, first match wins
SET_RULES=[
 (r'primer|undercoat|surface primer|paints for undercoating',"primer"),
 (r'contrast|speedpaint|xpress|instant colou?rs|one-coat',"contrast"),
 (r'\bwash|shade|quickshade|tone\b|dipping ink|wash fx',"wash"),
 (r'\bink|glaz|transparent|candy|inktensity|intensity|clear',"stain"),
 (r'metal|alchemy|liquid gold|chameleon|lascivus|super metallic|pearl|glitter',"metallic"),
 (r'technical|weathering|soil works|special fx|fx range|texture|gloss surface|matt surface',"effect"),
 (r'\bdry\b',"dry"),
 (r'\bair\b|airbrush|game air|model air',"air"),
 (r'spray|aerosol',"spray"),
]
METAL_NAME=r'\b(pearl|glitter|glittering|chameleon|gold|golden|silver|steel|iron|bronze|copper|brass|metal|metallic|chrome|alloy|mithril|pewter|gunmetal|platinum|aluminium|aluminum|titanium|leadbelcher|nickel|tin|electrum|oily)\b'
NOT_METAL=re.compile(r'pearl skin|glittering green|steel legion|ironjawz|gold(en)? (yellow|ochre|brown)|yellow ochre|golden (skin|blonde|brown|fur)|iron oxide|silver ?(grey|gray)', re.I)
WASH_NAME=r'\b(wash|washes|quickshade)\b'
CONTRAST_NAME=r'\b(contrast|speedpaint)\b'
def kind(r):
    if r['brand'] in REFERENCE: return "reference"
    s=r['set'].lower(); n=r['name']
    for pat,k in SET_RULES:
        if re.search(pat,s): 
            base=k; break
    else: base="paint"
    if base in ("paint","air","dry","spray"):
        if re.search(METAL_NAME,n,re.I) and not NOT_METAL.search(n): return "metallic" if base=="paint" else base+"+metallic"
        if re.search(WASH_NAME,n,re.I): return "wash"
        if re.search(CONTRAST_NAME,n,re.I): return "contrast"
    return base
def role(r):
    # Triad systems (Foundry): the dark, middle and light member of a set of three.
    m=re.search(r'\b(shade|light)$',r['name'],re.I)
    if r['brand']=='Foundry': return (m.group(1).lower() if m else 'base')
    return None
for r in rows: r['kind']=kind(r); r['role']=role(r)
c=collections.Counter(r['kind'] for r in rows)
print(sum(c.values()),'rows;',len(BAD),'without a colour;',sum(r['shifted'] for r in rows),'with shifted columns'); 
for k,v in c.most_common(): print(f"{k:16}{v}")
if OUT: json.dump(rows, open(OUT, 'w'))
if ARGS:
    for r in rows:
        if r['kind'] == ARGS[0] and (len(ARGS) < 2 or r['brand'] == ARGS[1]):
            print(r['brand'], '|', r['set'], '|', r['name'])
