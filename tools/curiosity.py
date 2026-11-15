"""Curiosity rover model for Walk mode.

Source: NASA Science 3D resources, "Curiosity Rover 3D Model" (24584_Curiosity_static.glb),
https://science.nasa.gov/resource/curiosity-rover-3d-model/ (file
https://assets.science.nasa.gov/content/dam/science/psd/mars/resources/gltf_files/24584_Curiosity_static.glb). NASA 3D
resources are free to use; credit NASA, imply no endorsement (NASA Images and Media Usage Guidelines).

The model (checked): one node 'MSL' with a translation, 9 primitives, 63,516 vertices, 51,305 triangles, 9 PNG
textures, metres, Y up. Measured on it: wheels touch y ~ 0; width 2.78 m (wheel contacts x -1.39..1.40); height 2.22 m (mast
head); the robotic arm overhangs the front wheels by ~0.95 m on +Z and the RTG sits behind the rear wheels on -Z, so the
rover faces +Z (front = arm end) and its right side is -X.

This tool bakes each vertex's base colour (texture x factor) so the app needs no textures (11.9 MB -> ~2.5 MB), recentres
the wheelbase on the origin (x, z) and keeps y = 0 at the wheel contacts.
Output: assets/curiosity.bin (pos f32x3, nrm f32x3, col u8x4 sRGB, idx u32) + assets/curiosity.json.
Usage: python tools/curiosity.py fetch | build | selftest
"""
import io
import json
import os
import struct
import sys
import urllib.request

import numpy as np

import config

SRC_URL = "https://assets.science.nasa.gov/content/dam/science/psd/mars/resources/gltf_files/24584_Curiosity_static.glb"
SRC_PAGE = "https://science.nasa.gov/resource/curiosity-rover-3d-model/"
RAW = os.path.join(config.RAW, "models", "Curiosity_static.glb")


def glb(path):
    b = open(path, "rb").read()
    cl = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20:20 + cl])
    return j, b, 20 + cl + 8


def accessor(j, b, off, i):
    a = j["accessors"][i]
    bv = j["bufferViews"][a["bufferView"]]
    n = {"VEC4": 4, "VEC3": 3, "VEC2": 2, "SCALAR": 1}[a["type"]]
    dt = {5126: "<f4", 5125: "<u4", 5123: "<u2", 5121: "u1"}[a["componentType"]]
    start = off + bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    arr = np.frombuffer(b, dt, a["count"] * n, start)
    return arr.reshape(-1, n) if n > 1 else arr


def bake(uv, img, factor):
    """Base colour per vertex: nearest texel (repeat wrap, glTF UV origin top-left) x factor, as sRGB bytes."""
    h, w = img.shape[:2]
    px = np.clip((np.mod(uv[:, 0], 1.0) * w).astype(int), 0, w - 1)
    py = np.clip((np.mod(uv[:, 1], 1.0) * h).astype(int), 0, h - 1)
    lin = (img[py, px] / 255.0) ** 2.2 * factor
    return np.clip(np.round(lin ** (1 / 2.2) * 255), 0, 255).astype(np.uint8)


def read(path):
    from PIL import Image
    j, b, off = glb(path)
    node = j["nodes"][j["scenes"][0]["nodes"][0]]
    assert "rotation" not in node and "scale" not in node and "matrix" not in node, "only a translation is handled"
    tr = np.array(node.get("translation", [0, 0, 0]), float)
    pos, nrm, col, idx = [], [], [], []
    base = 0
    for p in j["meshes"][node["mesh"]]["primitives"]:
        P = accessor(j, b, off, p["attributes"]["POSITION"]).astype(float) + tr
        N = accessor(j, b, off, p["attributes"]["NORMAL"]).astype(float)
        mat = j["materials"][p.get("material", 0)].get("pbrMetallicRoughness", {})
        factor = np.array(mat.get("baseColorFactor", [1, 1, 1, 1])[:3], float)
        if "baseColorTexture" in mat:
            t = mat["baseColorTexture"]
            img_i = j["textures"][t["index"]]["source"]
            bv = j["bufferViews"][j["images"][img_i]["bufferView"]]
            img = np.asarray(Image.open(io.BytesIO(b[off + bv.get("byteOffset", 0):off + bv.get("byteOffset", 0) + bv["byteLength"]])).convert("RGB"), float)
            uv = accessor(j, b, off, p["attributes"][f"TEXCOORD_{t.get('texCoord', 0)}"]).astype(float)
            C = bake(uv, img, factor)
        else:
            C = np.tile(np.clip(np.round(factor ** (1 / 2.2) * 255), 0, 255).astype(np.uint8), (len(P), 1))
        I = accessor(j, b, off, p["indices"]).astype(np.int64)
        pos.append(P); nrm.append(N); col.append(C); idx.append(I + base)
        base += len(P)
    return np.concatenate(pos), np.concatenate(nrm), np.concatenate(col), np.concatenate(idx)


def centre(P):
    """Shift x and z so the wheel contacts (lowest 0.25 m) are centred on the origin; y unchanged (0 = ground)."""
    low = P[P[:, 1] < P[:, 1].min() + 0.25]
    c = np.array([(low[:, 0].min() + low[:, 0].max()) / 2, 0.0, (low[:, 2].min() + low[:, 2].max()) / 2])
    return P - c, c


def build():
    P, N, C, I = read(RAW)
    P, c = centre(P)
    N = N / np.maximum(np.linalg.norm(N, axis=1, keepdims=True), 1e-9)
    blob = bytearray()
    off = {}
    for name, arr in (("pos", P.astype("<f4")), ("nrm", N.astype("<f4")), ("col", np.c_[C, np.full(len(C), 255, np.uint8)]),
                      ("idx", I.astype("<u4"))):
        off[name + "_offset"] = len(blob)
        blob += arr.tobytes()
        while len(blob) % 4:
            blob += bytes(1)
    open(os.path.join(config.ASSETS, "curiosity.bin"), "wb").write(bytes(blob))
    ext = (P.max(axis=0) - P.min(axis=0)).round(2).tolist()
    json.dump(dict(source=dict(name="NASA Science: Curiosity Rover 3D Model (24584_Curiosity_static.glb)", url=SRC_PAGE, file=SRC_URL,
                               credit="NASA/JPL-Caltech (3D model, NASA Science 3D resources); used without NASA endorsement",
                               terms="NASA 3D resources are free to use (NASA Images and Media Usage Guidelines)"),
                   frame="metres, Y up, faces +Z (arm end), right side -X; origin = centre of the wheel contacts, y 0 at the wheels",
                   extent_m=dict(x=ext[0], y=ext[1], z=ext[2]), recentred_by_m=c.round(4).tolist(),
                   vertices=int(len(P)), indices=int(len(I)), **off,
                   layout="pos f32x3, nrm f32x3, col u8x4 sRGB, idx u32",
                   method="tools/curiosity.py: node translation applied, base colour baked per vertex, wheelbase centred"),
              open(os.path.join(config.ASSETS, "curiosity.json"), "w"), indent=1)
    print(f"curiosity.bin {len(blob) / 1e6:.2f} MB: {len(P)} vertices, {len(I) // 3} triangles, extent {ext} m (x, y, z)")


def fetch():
    os.makedirs(os.path.dirname(RAW), exist_ok=True)
    with urllib.request.urlopen(SRC_URL, timeout=300) as r:
        open(RAW, "wb").write(r.read())
    print(f"downloaded {os.path.getsize(RAW)} bytes -> {RAW}")


def selftest():
    img = np.zeros((2, 2, 3))
    img[0, 1] = [255, 255, 255]
    c = bake(np.array([[0.75, 0.25], [0.25, 0.75]]), img, np.ones(3))
    assert c[0].tolist() == [255, 255, 255] and c[1].tolist() == [0, 0, 0], c
    P = np.array([[-1.0, 0.0, -2.0], [1.0, 0.0, 1.0], [0.0, 2.0, 5.0]])
    Q, cc = centre(P)
    assert np.allclose(cc, [0, 0, -0.5]) and np.allclose(Q[:, 1], P[:, 1])
    print("curiosity selftest: PASS")


if __name__ == "__main__":
    a = sys.argv[1:]
    {"selftest": selftest, "fetch": fetch, "build": build}.get(a[0] if a else "", lambda: print(__doc__))()
