"""Astronaut model for Walk mode's third-person view (the NASA 3D Resources "Astronaut" model).

Source: NASA 3D Resources, "Astronaut" (Astronaut.glb), mirrored at https://github.com/nasa/NASA-3D-Resources
("3D Models/Astronaut/Astronaut.glb"). NASA's 3D resources are "free to download and use"; credit NASA, imply no
endorsement (NASA Images and Media Usage Guidelines). The model is a static T-pose mesh (no skeleton, no animation,
Draco-compressed), so this tool prepares it for a procedural walk in the app:
  1 decode (DracoPy), convert the 3DS Z-up frame to Y-up (x, z, -y), scale to SUIT_HEIGHT_M (display only, ASSUMPTION);
  2 split triangles by centroid into parts: torso (incl. helmet, visor, backpack), left/right arm, left/right leg
    (cuts measured on the model, in model units, listed in CUTS);
  3 bake each vertex's colour from its base-colour texture x factor (so the app needs no texture), sRGB bytes;
  4 store each part relative to its pivot (shoulder/hip) so the app can rotate parts.
Output (format 2): ONE skinned mesh, see to_skinned() below: assets/astronaut.bin (positions, normals, colours,
joint indices, joint weights, indices) and assets/astronaut.json (offsets, joint pivots in metres, credit, source). The
model faces +Z, up +Y, its left is +X. (The format-1 part split is kept as to_parts() for reference; it tore at the cuts.)

Usage: python tools/astronaut.py fetch | build | selftest
"""
import io
import json
import os
import struct
import sys
import urllib.request

import numpy as np

import config

SRC_URL = "https://raw.githubusercontent.com/nasa/NASA-3D-Resources/master/3D%20Models/Astronaut/Astronaut.glb"
SRC_PAGE = "https://github.com/nasa/NASA-3D-Resources/tree/master/3D%20Models/Astronaut"
RAW = os.path.join(config.RAW, "models", "Astronaut.glb")
SUIT_HEIGHT_M = 1.85          # ASSUMPTION for display (the model has no stated scale)
# Part cuts in model units (Y-up frame, measured on this model: arms leave the torso at |x| ~14 above ~0.62 of the height;
# legs below ~0.40 of the height; the backpack lies behind fwd < -4.5)
CUTS = dict(arm_x=14.0, arm_up_min=5.0, leg_up_max=-5.0, backpack_fwd=-4.5)
PIVOTS = dict(arm_l=(14.5, 15.0, 3.0), arm_r=(-14.5, 15.0, 3.0), leg_l=(5.5, -5.0, 3.0), leg_r=(-5.5, -5.0, 3.0), torso=(0.0, -5.0, 0.0))
PARTS = ["torso", "arm_l", "arm_r", "leg_l", "leg_r"]


def part_of(cx, cy, cz):
    """Part name for a triangle centroid (model units, Y-up, facing +Z, left = +X)."""
    if abs(cx) > CUTS["arm_x"] and cy > CUTS["arm_up_min"]:
        return "arm_l" if cx > 0 else "arm_r"
    if cy < CUTS["leg_up_max"] and cz > CUTS["backpack_fwd"]:
        return "leg_l" if cx > 0 else "leg_r"
    return "torso"


def read_glb(path):
    import DracoPy
    from PIL import Image
    b = open(path, "rb").read()
    cl = struct.unpack("<I", b[12:16])[0]
    j = json.loads(b[20:20 + cl])
    off = 20 + cl
    bl = struct.unpack("<I", b[off:off + 4])[0]
    bin_ = b[off + 8:off + 8 + bl]

    def view(i):
        bv = j["bufferViews"][i]
        o = bv.get("byteOffset", 0)
        return bin_[o:o + bv["byteLength"]]

    prims = []
    for p in j["meshes"][0]["primitives"]:
        m = DracoPy.decode(view(p["extensions"]["KHR_draco_mesh_compression"]["bufferView"]))
        mat = j["materials"][p["material"]]["pbrMetallicRoughness"]
        tex = j["textures"][mat["baseColorTexture"]["index"]]
        img = Image.open(io.BytesIO(view(j["images"][tex["source"]]["bufferView"]))).convert("RGB")
        prims.append(dict(pos=np.asarray(m.points, float), nrm=np.asarray(m.normals, float), uv=np.asarray(m.tex_coord, float),
                          tri=np.asarray(m.faces, np.int64), img=np.asarray(img, float), factor=np.array(mat["baseColorFactor"][:3])))
    return prims


def bake_colour(uv, img, factor):
    """Base colour at each vertex (nearest texel, repeat wrap; glTF UV origin top-left), times the factor, as sRGB bytes."""
    h, w = img.shape[:2]
    u = np.mod(uv[:, 0], 1.0)
    v = np.mod(uv[:, 1], 1.0)
    px = np.clip((u * w).astype(int), 0, w - 1)
    py = np.clip((v * h).astype(int), 0, h - 1)
    lin = (img[py, px] / 255.0) ** 2.2 * factor          # texture is sRGB; the factor is linear (glTF)
    return np.clip(np.round(lin ** (1 / 2.2) * 255), 0, 255).astype(np.uint8)


def to_parts(prims, height_m=SUIT_HEIGHT_M):
    """Y-up frame, parts by triangle centroid, positions in metres relative to each part's pivot."""
    allp = np.concatenate([p["pos"] for p in prims])
    up = allp[:, 2]
    s = height_m / (up.max() - up.min())
    y0 = up.min()
    out = {k: dict(pos=[], nrm=[], col=[], idx=[]) for k in PARTS}
    for p in prims:
        P = np.c_[p["pos"][:, 0], p["pos"][:, 2], -p["pos"][:, 1]]           # x, up, fwd
        N = np.c_[p["nrm"][:, 0], p["nrm"][:, 2], -p["nrm"][:, 1]]
        C = bake_colour(p["uv"], p["img"], p["factor"])
        for t in p["tri"]:
            c = P[t].mean(axis=0)
            q = out[part_of(*c)]
            base = len(q["pos"])
            q["pos"].extend(P[t])
            q["nrm"].extend(N[t])
            q["col"].extend(C[t])
            q["idx"].extend([base, base + 1, base + 2])
    parts = {}
    for k, q in out.items():
        piv = np.array(PIVOTS[k], float)
        pos = (np.array(q["pos"], float).reshape(-1, 3) - piv) * s
        nrm = np.array(q["nrm"], float).reshape(-1, 3)
        nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-9)
        parts[k] = dict(pos=pos.astype("<f4"), nrm=nrm.astype("<f4"), col=np.c_[np.array(q["col"], np.uint8).reshape(-1, 3),
                                                                               np.full(len(q["pos"]), 255, np.uint8)],
                        idx=np.array(q["idx"], "<u4"), pivot_m=((piv - [0, y0, 0]) * s).round(4).tolist())
    return parts, s


# ---- v2 (the part split tore the arms): ONE connected mesh with joints, skinned on the GPU by the app ----------------
# The part split above left holes and torn edges where arms and legs were cut out of the T-pose. v2 keeps the suit whole and
# gives every vertex smooth joint weights, blended across the shoulders and hips, so a lowered or swinging arm bends the suit
# instead of tearing it. Joints: 0 torso (root), 1 arm_l, 2 arm_r, 3 leg_l, 4 leg_r, pivots as PIVOTS. Blend widths in
# model units (1 unit ~ 2.5 cm), measured on this model: arms leave the torso at |x| ~13-14 at shoulder height (y ~15),
# the torso is |x| <= 13 below y ~10; legs part below the hips (y ~ -5); the backpack lies behind z < -5.
JOINTS = ["torso", "arm_l", "arm_r", "leg_l", "leg_r"]
BLEND = dict(arm_x=(12.0, 17.0), arm_min_y=(5.0, 8.0), leg_y=(-3.0, -9.0), backpack_z=(-6.0, -4.0), side_x=(-1.5, 1.5))


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def joint_weights(P):
    """(n, 5) weights for model-unit points P (x left, y up, z forward); rows sum to 1."""
    x, y, z = P[:, 0], P[:, 1], P[:, 2]
    arm = smoothstep(*BLEND["arm_x"], np.abs(x)) * smoothstep(*BLEND["arm_min_y"], y)
    leg = smoothstep(*BLEND["leg_y"], y) * smoothstep(*BLEND["backpack_z"], z)
    side = smoothstep(*BLEND["side_x"], x)               # 1 = model's left (+x)
    w = np.zeros((len(P), 5))
    w[:, 1] = np.where(x > 0, arm, 0.0)
    w[:, 2] = np.where(x < 0, arm, 0.0)
    w[:, 3] = leg * side * (1 - arm)
    w[:, 4] = leg * (1 - side) * (1 - arm)
    w[:, 0] = np.clip(1.0 - w[:, 1:].sum(axis=1), 0.0, 1.0)
    return w / w.sum(axis=1, keepdims=True)


def to_skinned(prims, height_m=SUIT_HEIGHT_M):
    """One mesh: shared vertices (deduplicated on position, normal and colour), metres from the soles, joint weights."""
    allp = np.concatenate([p["pos"] for p in prims])
    s = height_m / (allp[:, 2].max() - allp[:, 2].min())
    y0 = allp[:, 2].min()
    keys = {}
    pos, nrm, col, mu, idx = [], [], [], [], []
    for p in prims:
        P = np.c_[p["pos"][:, 0], p["pos"][:, 2], -p["pos"][:, 1]]
        N = np.c_[p["nrm"][:, 0], p["nrm"][:, 2], -p["nrm"][:, 1]]
        N /= np.maximum(np.linalg.norm(N, axis=1, keepdims=True), 1e-9)
        C = bake_colour(p["uv"], p["img"], p["factor"])
        for t in p["tri"]:
            for v in t:
                k = (tuple(np.round(P[v], 3)), tuple(np.round(N[v], 2)), tuple(C[v]))
                i = keys.get(k)
                if i is None:
                    i = keys[k] = len(pos)
                    pos.append(P[v]); nrm.append(N[v]); col.append(C[v]); mu.append(P[v])
                idx.append(i)
    mu = np.array(mu)
    W = joint_weights(mu)
    order = np.argsort(-W, axis=1)[:, :4]                 # the four largest influences
    jw = np.take_along_axis(W, order, axis=1)
    jw /= jw.sum(axis=1, keepdims=True)
    P = (mu - [0, y0, 0]) * s
    piv = {k: ((np.array(PIVOTS[k], float) - [0, y0, 0]) * s).round(4).tolist() for k in JOINTS}
    return dict(pos=P.astype("<f4"), nrm=np.array(nrm, "<f4"),
                col=np.c_[np.array(col, np.uint8), np.full(len(col), 255, np.uint8)],
                joints=order.astype("<u2"), weights=jw.astype("<f4"), idx=np.array(idx, "<u4"), pivots=piv), s


def write_skinned(m, out_dir, scale):
    blob = bytearray()
    off = {}
    for field in ("pos", "nrm", "col", "joints", "weights", "idx"):
        off[field + "_offset"] = len(blob)
        blob += m[field].tobytes()
        while len(blob) % 4:
            blob += b"\0"
    open(os.path.join(out_dir, "astronaut.bin"), "wb").write(bytes(blob))
    json.dump(dict(format=2, source=dict(name="NASA 3D Resources: Astronaut (Astronaut.glb)", url=SRC_PAGE,
                                         credit="NASA 3D Resources (model); used without NASA endorsement",
                                         terms="NASA 3D resources are free to download and use (NASA Images and Media Usage Guidelines)"),
                   frame="Y up, faces +Z, model's left = +X; positions in metres from the soles",
                   suit_height_m=SUIT_HEIGHT_M, suit_height_label="ASSUMPTION (display only; the model has no stated scale)",
                   model_units_per_m=round(1 / scale, 4), vertices=int(len(m["pos"])), indices=int(len(m["idx"])), **off,
                   joints=[dict(name=k, pivot_m=m["pivots"][k]) for k in JOINTS], blend_model_units=BLEND,
                   layout="pos f32x3, nrm f32x3, col u8x4 sRGB, joints u16x4, weights f32x4 (sum 1), idx u32",
                   method="tools/astronaut.py v2: Draco decode, Y-up, vertices shared, smooth joint weights (skinned in the app)"),
              open(os.path.join(out_dir, "astronaut.json"), "w"), indent=1)


def write(parts, out_dir, scale):
    blob = bytearray()
    meta = []
    for k in PARTS:
        q = parts[k]
        entry = dict(name=k, vertices=int(len(q["pos"])), indices=int(len(q["idx"])), pivot_m=q["pivot_m"])
        for field in ("pos", "nrm", "col", "idx"):
            entry[f"{field}_offset"] = len(blob)
            blob += q[field].tobytes()
            while len(blob) % 4:
                blob += b"\0"
        meta.append(entry)
    open(os.path.join(out_dir, "astronaut.bin"), "wb").write(bytes(blob))
    json.dump(dict(source=dict(name="NASA 3D Resources: Astronaut (Astronaut.glb)", url=SRC_PAGE,
                               credit="NASA 3D Resources (model); used without NASA endorsement",
                               terms="NASA 3D resources are free to download and use (NASA Images and Media Usage Guidelines)"),
                   frame="Y up, faces +Z, model's left = +X; positions in metres relative to each part's pivot; pivot_m from the soles",
                   suit_height_m=SUIT_HEIGHT_M, suit_height_label="ASSUMPTION (display only; the model has no stated scale)",
                   model_units_per_m=round(1 / scale, 4), cuts_model_units=CUTS, parts=meta,
                   method="tools/astronaut.py: Draco decode, Y-up, parts by triangle centroid, base colour baked per vertex"),
              open(os.path.join(out_dir, "astronaut.json"), "w"), indent=1)


def fetch():
    os.makedirs(os.path.dirname(RAW), exist_ok=True)
    with urllib.request.urlopen(SRC_URL, timeout=120) as r:
        open(RAW, "wb").write(r.read())
    print(f"downloaded {os.path.getsize(RAW)} bytes -> {RAW}")


def build():
    m, s = to_skinned(read_glb(RAW))
    write_skinned(m, config.ASSETS, s)
    share = (m["weights"][:, 0] < 0.999).mean()
    print(f"  {len(m['pos'])} shared vertices, {len(m['idx']) // 3} triangles; {share:.1%} of vertices move with an arm or leg")
    print("astronaut.bin / astronaut.json (format 2, skinned) written to assets/ (run build_bundle rehash)")


def selftest():
    # a box figure: torso block, arms out sideways at shoulder height, two legs, backpack behind
    assert part_of(25.0, 15.0, 0.0) == "arm_l" and part_of(-25.0, 15.0, 0.0) == "arm_r"
    assert part_of(5.0, -20.0, 3.0) == "leg_l" and part_of(-5.0, -20.0, 3.0) == "leg_r"
    assert part_of(0.0, 20.0, 0.0) == "torso", "head/chest"
    assert part_of(3.0, -8.0, -10.0) == "torso", "backpack bottom behind the hips stays torso"
    assert part_of(15.0, 0.0, 0.0) == "torso", "hip-level wide point is not an arm"
    img = np.zeros((2, 2, 3))
    img[0, 0] = [255, 0, 0]
    c = bake_colour(np.array([[0.1, 0.1], [0.9, 0.9]]), img, np.array([1.0, 1.0, 1.0]))
    assert c[0].tolist() == [255, 0, 0] and c[1].tolist() == [0, 0, 0], c
    # frame + scale on a synthetic two-triangle "model": height maps to SUIT_HEIGHT_M, the pivot is relative to the soles
    tri = np.array([[0, 1, 2], [3, 4, 5]])
    pos = np.array([[0, 0, -37.0], [1, 0, -37.0], [0, 0, -30.0], [0, 0, 37.0], [1, 0, 37.0], [0, 0, 30.0]])
    pr = [dict(pos=pos, nrm=np.tile([0, 0, 1.0], (6, 1)), uv=np.zeros((6, 2)), tri=tri, img=np.full((1, 1, 3), 255.0), factor=np.ones(3))]
    parts, s = to_parts(pr, 1.85)
    assert abs(s * 74.0 - 1.85) < 1e-9
    assert parts["torso"]["pos"].shape[0] == 3 and parts["leg_l"]["pos"].shape[0] == 3, {k: len(v["pos"]) for k, v in parts.items()}
    assert abs(parts["torso"]["pivot_m"][1] - (-5.0 + 37.0) * s) < 1e-3
    # v2 weights: hand -> arm, chest -> torso, foot -> leg, backpack -> torso, smooth (no jump) across the shoulder
    W = joint_weights(np.array([[30.0, 15.0, 0.0], [-30.0, 15.0, 0.0], [0.0, 15.0, 0.0], [6.0, -30.0, 3.0], [-6.0, -30.0, 3.0],
                                [0.0, -8.0, -12.0]]))
    assert W[0, 1] > 0.99 and W[1, 2] > 0.99 and W[2, 0] > 0.99 and W[3, 3] > 0.99 and W[4, 4] > 0.99 and W[5, 0] > 0.99, W.round(2)
    xs = np.linspace(10, 20, 101)
    wa = joint_weights(np.c_[xs, np.full_like(xs, 15.0), np.zeros_like(xs)])[:, 1]
    assert np.all(np.diff(wa) >= -1e-12) and np.max(np.diff(wa)) < 0.05, "arm weight rises smoothly across the shoulder"
    assert np.allclose(joint_weights(np.random.default_rng(1).uniform(-35, 35, (200, 3))).sum(axis=1), 1.0)
    print("astronaut selftest: PASS")


if __name__ == "__main__":
    a = sys.argv[1:]
    {"selftest": selftest, "fetch": fetch, "build": build}.get(a[0] if a else "", lambda: print(__doc__))()
