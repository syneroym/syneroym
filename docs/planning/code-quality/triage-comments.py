import re,subprocess,json,collections,sys

# One citation token, in any of the seven families.
CIT = re.compile(r"""(?x)
    \b M0\d[A-Z]? (?:\s+(?:Slice\s+)?[A-Z]?\d+[a-z]?)*      # M05A A5a / M04A Slice B7a
  | \b Slice\s+[A-Z]?\d+[a-z]?                              # Slice B7a
  | \b D-[0-9A-Z]{1,4}-\d+                                  # D-B1-9
  | \b WO\d
  | § \s? [\d.]+                                            # §11.2
  | \b review\s+(?:finding|round)\s+[A-Z]?-?\d+
  | \b (?:status|task)\.md
  | \b implementation-plan
  | \b [Tt]est\s+\d{1,3}\b                                  # Test 97
  | \b matrix\s+row\s+\d                                    # failure-matrix row 13
  | \b exit\s+criteri(?:on|a)\b
""")
ADRSEC = re.compile(r'ADR-\d{4}[^.]{0,12}§')
PREFIX = re.compile(r'^(\s*)(///?!?|//|\*/?|/\*+!?)\s?')

def blocks(path):
    """Yield (start_line, [raw lines]) for each run of consecutive comment lines."""
    lines = open(path, encoding='utf-8', errors='replace').read().split('\n')
    cur, start = [], None
    for i, l in enumerate(lines, 1):
        if re.match(r'^\s*(//|/\*|\*)', l):
            if start is None: start = i
            cur.append(l)
        else:
            if cur: yield start, cur
            cur, start = [], None
    if cur: yield start, cur

def classify(text, raw):
    """text = joined prose of the block."""
    hits = [m for m in CIT.finditer(text)
            if not (m.group(0).startswith('§') and ADRSEC.search(text[max(0,m.start()-24):m.end()]))]
    if not hits: return None, 0
    kinds = set()
    for m in hits:
        s, e = m.start(), m.end()
        before, after = text[:s], text[e:]
        # C: citation is the grammatical subject -- "A5e's loop", "D-S1-6's other half"
        if after[:2] == "'s":
            kinds.add('C_subject'); continue
        # find enclosing parenthesis, if any
        op = before.rfind('('); cl = before.rfind(')')
        inparen = op > cl
        if inparen:
            close = after.find(')')
            inner_before = before[op+1:]
            inner_after = after[:close] if close >= 0 else after
            if not inner_before.strip() and not inner_after.strip():
                kinds.add('A_whole_paren')                       # "(D-B1-6)"
            elif not inner_before.strip() and inner_after.lstrip()[:1] in (':', ','):
                kinds.add('B_paren_prefix')                      # "(D-A1-2: real content)"
            elif not inner_after.strip() and inner_before.rstrip()[-1:] in (',', ';'):
                kinds.add('B_paren_suffix')                      # "(real content, M05B B1)"
            else:
                kinds.add('D_needs_read')
        else:
            # sentence-leading citation followed by ':' -- "M05B B1 review: ..."
            if after.lstrip()[:1] == ':' and (not before.strip() or before.rstrip()[-1:] in ('.', '')):
                kinds.add('C_subject')
            else:
                kinds.add('D_needs_read')
    order = ['D_needs_read','C_subject','B_paren_suffix','B_paren_prefix','A_whole_paren']
    for o in order:
        if o in kinds: return o, len(hits)
    return 'D_needs_read', len(hits)

tracked=[p for p in subprocess.run(['git','ls-files','*.rs'],capture_output=True,text=True)
         .stdout.split() if not p.endswith('bindings.rs')]
cat=collections.Counter(); citcount=collections.Counter(); percrate=collections.defaultdict(collections.Counter)
rows=[]
for p in tracked:
    for start, raw in blocks(p):
        text=' '.join(PREFIX.sub('', l).strip() for l in raw).strip()
        k,n = classify(text, raw)
        if not k: continue
        cat[k]+=1; citcount[k]+=n
        percrate['/'.join(p.split('/')[:2])][k]+=1
        rows.append({'file':p,'line':start,'lines':len(raw),'class':k,'citations':n,'text':text[:260]})
json.dump(rows, open('/tmp/claude-501/triage/triage.json','w'), indent=1)
tot=sum(cat.values()); totc=sum(citcount.values())
print(f"comment BLOCKS containing a citation: {tot}   (citations inside them: {totc})\n")
LBL={'A_whole_paren':'A  strip whole parenthetical      ',
     'B_paren_prefix':'B1 strip citation, keep paren body',
     'B_paren_suffix':'B2 strip trailing citation in paren',
     'C_subject':'C  citation is the subject -- rewrite',
     'D_needs_read':'D  needs a human read              '}
for k in ['A_whole_paren','B_paren_prefix','B_paren_suffix','C_subject','D_needs_read']:
    print(f"  {LBL[k]}  {cat[k]:4d} blocks  ({citcount[k]:4d} citations)  {cat[k]*100//max(tot,1):3d}%")
auto=cat['A_whole_paren']+cat['B_paren_prefix']+cat['B_paren_suffix']
print(f"\n  mechanical (A+B): {auto} blocks, {auto*100//max(tot,1)}%")
print(f"  judgement (C+D):  {tot-auto} blocks, {(tot-auto)*100//max(tot,1)}%")
print("\nper crate (blocks):")
print(f"  {'crate':<32}{'A':>5}{'B1':>5}{'B2':>5}{'C':>5}{'D':>5}{'tot':>6}")
for c,cc in sorted(percrate.items(), key=lambda x:-sum(x[1].values()))[:14]:
    t=sum(cc.values())
    print(f"  {c:<32}{cc['A_whole_paren']:>5}{cc['B_paren_prefix']:>5}{cc['B_paren_suffix']:>5}{cc['C_subject']:>5}{cc['D_needs_read']:>5}{t:>6}")
