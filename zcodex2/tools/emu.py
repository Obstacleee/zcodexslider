import os, re, sys, importlib.util

here = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("sd", os.path.join(here, "strdec.py"))
sd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sd)

SRC = os.path.join(os.environ["TEMP"], "ilspy", "src", "QgZ9AvcBiZUprUQlRpc", "p7oYGLcbW1DI5lpOXbA.cs")
lines = open(SRC, encoding="utf8").read().split("\n")

# locate the second `try` (the one with ReadBytes) up to end_IL label
start = next(i for i, l in enumerate(lines) if "binaryReader.BaseStream.Position = 0L" in l)
end = next(i for i, l in enumerate(lines) if l.strip().startswith("end_IL_"))
body = [l.strip() for l in lines[start:end] if l.strip()]

# linearize
prog = []          # (op, arg...)
stack = []
cnt = 0
for l in body:
    if l == "{":
        continue
    if l.startswith("if ("):
        cond = l[4:-1] if l.endswith(")") else None
        cnt += 1
        prog.append(("jmpifnot", cond, f"__end{cnt}"))
        stack.append(f"__end{cnt}")
        continue
    if l == "}":
        prog.append(("label", stack.pop()))
        continue
    m = re.fullmatch(r"(IL_[0-9a-f]{4}):", l)
    if m:
        prog.append(("label", m.group(1)))
        continue
    m = re.fullmatch(r"goto (\w+);", l)
    if m:
        prog.append(("goto", m.group(1)))
        continue
    prog.append(("stmt", l))
assert not stack, stack
labels = {}
for i, ins in enumerate(prog):
    if ins[0] == "label":
        labels[ins[1]] = i


def pycond(c):
    c = c.replace("GUT8n9JTxGwYl9mPRb()", "True").replace("RmcwTo2h7nw4XGqBnf()", "None")
    c = re.sub(r"!\s*True", "False", c)
    c = c.replace("!= null", "is not None").replace("== null", "is None")
    return c


nums = {}
arrays = {}

result = {}


class V(dict):
    pass


def run_stmt(s):
    s = s.rstrip(";")
    m = re.fullmatch(r"byte\[\] (\w+) = default\(byte\[\]\)", s)
    if m:
        arrays.setdefault(m.group(1), None)  # decompiler artifact, not a runtime assignment
        return
    m = re.fullmatch(r"(?:byte\[\] )?(\w+) = new byte\[(\d+)\]", s)
    if m:
        arrays[m.group(1)] = bytearray(int(m.group(2)))
        return
    m = re.fullmatch(r"byte\[\] (\w+) = (array\d+)", s)
    if m:
        arrays[m.group(1)] = arrays[m.group(2)]
        return
    m = re.fullmatch(r"(\w+)\[(\d+)\] = \(byte\)(num\d*)", s)
    if m:
        arr = arrays[m.group(1)]
        arr[int(m.group(2))] = nums[m.group(3)] & 255
        return
    m = re.fullmatch(r"(\w+)\[(\d+)\] = (\d+)", s)
    if m:
        arr = arrays[m.group(1)]
        arr[int(m.group(2))] = int(m.group(3)) & 255
        return
    m = re.fullmatch(r"(?:int )?(num\d*) = (.+)", s)
    if m and "binaryReader" not in s and "BitConverter" not in s:
        nums[m.group(1)] = eval(m.group(2), {}, nums)
        return
    if s.startswith("byte[] array2 = binaryReader.ReadBytes"):
        arrays["array2"] = bytearray(open(sd.RES, "rb").read())
        return
    if s.startswith("qOO4Q8r0F0d = new p7oYGLcbW1DI5lpOXbA("):
        m = re.search(r"\((\w+), (\w+)\)\.Q9d4QGoYVcO\((\w+)\)", s)
        result["key"] = bytes(arrays[m.group(1)])
        result["iv"] = bytes(arrays[m.group(2)])
        result["data"] = bytes(arrays[m.group(3)])
        return
    if s.startswith("binaryReader.BaseStream.Position"):
        return
    raise RuntimeError("unhandled: " + s)


import collections
trace = collections.deque(maxlen=400)
pc = 0
steps = 0
while pc < len(prog):
    steps += 1
    if steps > 5_000_000:
        raise RuntimeError("too many steps")
    ins = prog[pc]
    op = ins[0]
    trace.append((pc, ins))
    if op == "label":
        pc += 1
    elif op == "goto":
        if ins[1].startswith("end_"):
            break
        pc = labels[ins[1]]
    elif op == "jmpifnot":
        cond = pycond(ins[1])
        if eval(cond, {}, nums):
            pc += 1
        else:
            pc = labels[ins[2]]
    else:
        try:
            run_stmt(ins[1])
        except Exception:
            for t in trace: print(t)
            raise
        pc += 1
        if "key" in result:
            break

print("steps", steps)
print("key", list(result["key"]))
print("iv", list(result["iv"]))
tab = sd.decrypt(result["data"], result["key"])
open(os.path.join(here, "strtab.bin"), "wb").write(tab)
import struct
for i in [69492, 86536, 87080, 87148, 86404, 86466, 86502, 87174]:
    n = struct.unpack_from("<i", tab, i)[0]
    print(i, n, ascii(tab[i + 4:i + 4 + n].decode("utf-16-le", "replace")) if 0 <= n < 200 else "bad")
