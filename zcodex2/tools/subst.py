import os, re, struct, json

here = os.path.dirname(os.path.abspath(__file__))
tab = open(os.path.join(here, "strtab.bin"), "rb").read()
src = os.path.join(os.environ["TEMP"], "ilspy", "src")
out = os.path.join(os.environ["TEMP"], "ilspy", "src_dec")

cache = {}


def get(i):
    if i not in cache:
        try:
            n = struct.unpack_from("<i", tab, i)[0]
            cache[i] = tab[i + 4:i + 4 + n].decode("utf-16-le") if 0 <= n < 20000 else "<bad>"
        except Exception:
            cache[i] = "<err>"
    return cache[i]


pat = re.compile(r"p7oYGLcbW1DI5lpOXbA\.oT24QCCWPaF\((-?\d+)\)")


def lit(m):
    return json.dumps(get(int(m.group(1))), ensure_ascii=False)


count = 0
for root, _, files in os.walk(src):
    for f in files:
        if not f.endswith(".cs"):
            continue
        p = os.path.join(root, f)
        s = open(p, encoding="utf8", errors="replace").read()
        t, n = pat.subn(lit, s)
        if n:
            rel = os.path.relpath(p, src)
            q = os.path.join(out, rel)
            os.makedirs(os.path.dirname(q), exist_ok=True)
            open(q, "w", encoding="utf8").write(t)
            count += n
print("replaced", count)

# plain list of table strings reachable by scanning the offsets referenced
ids = set()
for root, _, files in os.walk(src):
    for f in files:
        if f.endswith(".cs"):
            ids.update(int(x) for x in pat.findall(open(os.path.join(root, f), encoding="utf8", errors="replace").read()))
with open(os.path.join(here, "strings.txt"), "w", encoding="utf8") as fh:
    for i in sorted(ids):
        fh.write(f"{i}\t{get(i)!r}\n")
print("distinct ids", len(ids))
