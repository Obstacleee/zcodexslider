import os, struct, sys, re

M = 0xFFFFFFFF
RES = os.path.join(os.environ["TEMP"], "ilspy", "src", "bT8dMVXggXiZFg3Mkm.b9awHiMBuf1viDDo7b")
KEY = bytes([123, 5, 74, 12, 244, 156, 221, 154, 121, 221, 183, 41, 121, 65, 9, 43, 67, 81, 23, 43,
             74, 63, 64, 23, 95, 185, 226, 244, 45, 194, 211, 43])


def rotr(x, n):
    return ((x >> n) | (x << (32 - n))) & M


def xtt(p):
    num = p
    num2 = 973202305
    num3 = 1582787682
    num4 = 1577548636
    num5 = 332884210
    num6 = ((num4 * 1313243236) & M) | 1
    num3 = ((num3 * num3) & M) % num6
    num2 = (-502326134 - num3) & M
    num8 = ((num3 * 183835789) & M) | 1
    num4 = ((num4 * num4) & M) % num8
    num9 = rotr(num5, 6) ^ num3
    num10 = num9 & 0x0F0F0F0F
    num5 = ((num9 & 0xF0F0F0F0) >> 4) | ((num10 << 4) & M)
    num ^= (num << 3) & M
    num = (num + num2) & M
    num ^= (num << 11) & M
    num = (num + num4) & M
    num ^= num >> 27
    num = (num + num5) & M
    return ((((num2 << 11) & M) - num3 & M) ^ num4) - num & M


def decrypt(data, key):
    n = len(data)
    rem = n % 4
    words = n // 4 + (1 if rem else 0)
    kw = len(key) // 4
    out = bytearray(n)
    st = 0
    for i in range(words):
        k = struct.unpack_from("<I", key, (i % kw) * 4)[0]
        if i == words - 1 and rem:
            v = 0
            for j in range(rem):
                if j > 0:
                    v <<= 8
                v |= data[n - 1 - j]
            st = (st + k) & M
            st = (st + xtt(st)) & M
            x = st ^ v
            mask = 255
            sh = 0
            for kk in range(rem):
                if kk > 0:
                    mask <<= 8
                    sh += 8
                out[i * 4 + kk] = (x & mask) >> sh
        else:
            v = struct.unpack_from("<I", data, i * 4)[0]
            st = (st + k) & M
            st = (st + xtt(st)) & M
            struct.pack_into("<I", out, i * 4, st ^ v)
    return bytes(out)


def get(tab, off):
    n = struct.unpack_from("<i", tab, off)[0]
    return tab[off + 4: off + 4 + n].decode("utf-16-le", "replace")


if __name__ == "__main__":
    raw = open(RES, "rb").read()
    tab = decrypt(raw, KEY)
    print(len(raw))
    for i in [69492, 86536, 87080, 87148, 86404, 86466, 86502, 87174, 77554, 85154]:
        try:
            print(i, repr(get(tab, i))[:100])
        except Exception as e:
            print(i, "ERR", e)
