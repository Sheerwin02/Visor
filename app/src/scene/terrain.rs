//! Terrain mesh from heightmap.bin (bundle contract: float32 LE, row-major, row 0 = north, 1 m cells, heights above z_offset_m).
//! World axes: x = e_m (east), y = height, z = -n_m (north is -z). One vertex per cell centre; u = (col+0.5)/W, v = (row+0.5)/H.
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub cell: f32,
    pub z: Vec<f32>,
    /// highest height in z (cached: the ground ray uses it every frame)
    pub top: f32,
}

impl Grid {
    pub fn from_bytes(bytes: &[u8], w: usize, h: usize, cell: f32) -> Result<Self, String> {
        if bytes.len() != w * h * 4 {
            return Err(format!("heightmap.bin has {} bytes, expected {}", bytes.len(), w * h * 4));
        }
        let z: Vec<f32> = bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let top = z.iter().cloned().fold(f32::MIN, f32::max);
        Ok(Grid { w, h, cell, z, top })
    }
    fn at(&self, c: isize, r: isize) -> f32 {
        let c = c.clamp(0, self.w as isize - 1) as usize;
        let r = r.clamp(0, self.h as isize - 1) as usize;
        self.z[r * self.w + c]
    }
    /// World position of cell (row, col) centre.
    pub fn pos(&self, r: usize, c: usize) -> Vec3 {
        let e = (c as f32 + 0.5) * self.cell;
        let n = (self.h as f32 - r as f32 - 0.5) * self.cell;
        Vec3::new(e, self.z[r * self.w + c], -n)
    }
    /// Height (above z_offset) at map position (e east, n north, metres), bilinear between cell centres; clamped at the edges.
    pub fn height_at(&self, e: f32, n: f32) -> f32 {
        let x = e / self.cell - 0.5;
        let y = (self.h as f32 - 0.5) - n / self.cell; // fractional row (row 0 = north)
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (c, r) = (x0 as isize, y0 as isize);
        let top = self.at(c, r) * (1.0 - fx) + self.at(c + 1, r) * fx;
        let bot = self.at(c, r + 1) * (1.0 - fx) + self.at(c + 1, r + 1) * fx;
        top * (1.0 - fy) + bot * fy
    }
    pub fn min_height(&self) -> f32 {
        self.z.iter().cloned().fold(f32::MAX, f32::min)
    }
    pub fn max_height(&self) -> f32 {
        self.top
    }
    /// True if world point (x, z) lies over the grid.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let (e, n) = (x, -z);
        e >= 0.0 && n >= 0.0 && e <= self.w as f32 * self.cell && n <= self.h as f32 * self.cell
    }
    /// Distance along the ray `origin + t * dir` (dir unit length) to where it first meets the terrain, within `max_t`;
    /// None if it misses (sky, or off the map). Starts where the ray drops below the highest point, marches in half-cell
    /// steps, then refines by bisection.
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<f32> {
        let above = |p: Vec3| p.y > self.height_at(p.x, -p.z);
        let mut t = if origin.y > self.top {
            if dir.y >= 0.0 {
                return None;
            }
            (origin.y - self.top) / -dir.y
        } else {
            0.0
        };
        let step = self.cell * 0.5;
        let mut prev = t;
        while t <= max_t {
            let p = origin + dir * t;
            if p.y > self.top && dir.y >= 0.0 {
                return None; // climbing above everything
            }
            if self.contains(p.x, p.z) && !above(p) {
                let (mut lo, mut hi) = (prev, t);
                for _ in 0..20 {
                    let mid = (lo + hi) / 2.0;
                    if above(origin + dir * mid) { lo = mid } else { hi = mid }
                }
                return Some(hi);
            }
            prev = t;
            t += step;
        }
        None
    }
    /// Radius of a sphere around centre() that contains the whole terrain (kept for tests and future framing).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn bounding_radius(&self) -> f32 {
        let hx = self.w as f32 * self.cell / 2.0;
        let hz = self.h as f32 * self.cell / 2.0;
        let hy = (self.max_height() - self.min_height()) / 2.0;
        (hx * hx + hz * hz + hy * hy).sqrt()
    }
    /// Vertical walls around the edge down to `depth` below the lowest point, so the cropped terrain reads as a solid block.
    pub fn skirt(&self, depth: f32) -> Mesh {
        let (w, h) = (self.w, self.h);
        let mut ring: Vec<(usize, usize)> = Vec::new();
        ring.extend((0..w).map(|c| (0, c))); // north edge, west to east
        ring.extend((1..h).map(|r| (r, w - 1))); // east edge, north to south
        ring.extend((0..w - 1).rev().map(|c| (h - 1, c))); // south edge, east to west
        ring.extend((1..h - 1).rev().map(|r| (r, 0))); // west edge, south to north
        let floor = self.min_height() - depth;
        let mut pos = Vec::with_capacity(ring.len() * 2);
        for &(r, c) in &ring {
            let p = self.pos(r, c);
            pos.push(p.to_array());
            pos.push([p.x, floor, p.z]);
        }
        let n = ring.len() as u32;
        let mut idx = Vec::with_capacity(ring.len() * 6);
        for i in 0..n {
            let j = (i + 1) % n;
            let (t0, b0, t1, b1) = (2 * i, 2 * i + 1, 2 * j, 2 * j + 1);
            idx.extend_from_slice(&[t0, b0, t1, t1, b0, b1]);
        }
        let normals = vec![[0.0, 0.0, 1.0]; pos.len()]; // unlit, double-sided material: normals unused for shading
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        m.insert_indices(Indices::U32(idx));
        m
    }

    /// Heights box-smoothed over 3 x 3 cells (edge cells keep their own height, so planes stay planes). Used only for the
    /// lighting normals:
    /// the 1 m DEM is noise at pixel scale (gate FAIL), and Walk mode's sun would show it as pitting.
    pub fn smoothed3(&self) -> Vec<f32> {
        let (w, h) = (self.w as isize, self.h as isize);
        let mut out = vec![0.0; self.z.len()];
        for r in 0..h {
            for c in 0..w {
                if r == 0 || c == 0 || r == h - 1 || c == w - 1 {
                    out[(r * w + c) as usize] = self.z[(r * w + c) as usize];
                    continue;
                }
                let (mut s, mut n) = (0.0, 0.0);
                for dr in -1..=1 {
                    for dc in -1..=1 {
                        let (rr, cc) = (r + dr, c + dc);
                        if rr >= 0 && rr < h && cc >= 0 && cc < w {
                            s += self.z[(rr * w + cc) as usize];
                            n += 1.0;
                        }
                    }
                }
                out[(r * w + c) as usize] = s / n;
            }
        }
        out
    }

    pub fn mesh(&self) -> Mesh {
        let (w, h) = (self.w, self.h);
        let sm = self.smoothed3();
        let at_s = |c: isize, r: isize| sm[(r.clamp(0, h as isize - 1) as usize) * w + c.clamp(0, w as isize - 1) as usize];
        let mut pos = Vec::with_capacity(w * h);
        let mut nor = Vec::with_capacity(w * h);
        let mut uv = Vec::with_capacity(w * h);
        for r in 0..h {
            for c in 0..w {
                pos.push(self.pos(r, c).to_array());
                let (ci, ri) = (c as isize, r as isize);
                // dz/deast and dz/dnorth by central differences (row index runs south)
                // on the 3 m smoothed heights (lighting only; positions stay at the DEM's own heights)
                let dze = (at_s(ci + 1, ri) - at_s(ci - 1, ri)) / (2.0 * self.cell);
                let dzn = (at_s(ci, ri - 1) - at_s(ci, ri + 1)) / (2.0 * self.cell);
                // surface y = z(x, -zw): normal = (-dz/dx, 1, -dz/dzw) with zw = -n  =>  (-dze, 1, +dzn)
                nor.push(Vec3::new(-dze, 1.0, dzn).normalize().to_array());
                uv.push([(c as f32 + 0.5) / w as f32, (r as f32 + 0.5) / h as f32]);
            }
        }
        let mut idx = Vec::with_capacity((w - 1) * (h - 1) * 6);
        for r in 0..h - 1 {
            for c in 0..w - 1 {
                let a = (r * w + c) as u32;
                let b = a + 1;
                let d = a + w as u32;
                let e = d + 1;
                // counter-clockwise seen from above (+y)
                idx.extend_from_slice(&[a, d, b, b, d, e]);
            }
        }
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
        m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        m.insert_indices(Indices::U32(idx));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_finds_the_ground_and_misses_the_sky() {
        let mut b = Vec::new();
        for _ in 0..20 * 20 {
            b.extend_from_slice(&5.0f32.to_le_bytes());
        }
        let g = Grid::from_bytes(&b, 20, 20, 1.0).unwrap();
        // straight down from 50 m over the middle: ground at 5 m, 45 m away
        let t = g.ray_hit(Vec3::new(10.0, 50.0, -10.0), Vec3::NEG_Y, 1000.0).unwrap();
        assert!((t - 45.0).abs() < 0.02, "{t}");
        // 45 deg down from outside the map: meets the plane at y = 5
        let d = Vec3::new(1.0, -1.0, 0.0).normalize();
        let t = g.ray_hit(Vec3::new(-20.0, 35.0, -10.0), d, 1000.0).unwrap();
        let p = Vec3::new(-20.0, 35.0, -10.0) + d * t;
        assert!((p.y - 5.0).abs() < 0.02 && (p.x - 10.0).abs() < 0.05, "{p:?}");
        assert!(g.ray_hit(Vec3::new(10.0, 50.0, -10.0), Vec3::Y, 1000.0).is_none(), "looking up hits nothing");
        assert!(g.ray_hit(Vec3::new(10.0, 50.0, -10.0), Vec3::X, 1000.0).is_none(), "level above the top hits nothing");
    }

    #[test]
    fn plane_rising_east_has_tilted_normal_and_north_is_minus_z() {
        let (w, h) = (4usize, 3usize);
        let mut bytes = Vec::new();
        for _r in 0..h {
            for c in 0..w {
                bytes.extend_from_slice(&(c as f32).to_le_bytes()); // rises 1 m per 1 m east (45 deg)
            }
        }
        let g = Grid::from_bytes(&bytes, w, h, 1.0).unwrap();
        assert_eq!(g.pos(0, 0), Vec3::new(0.5, 0.0, -2.5)); // row 0 is the north edge
        assert_eq!(g.pos(2, 3), Vec3::new(3.5, 3.0, -0.5));
        let m = g.mesh();
        assert_eq!(m.count_vertices(), w * h);
        let n = Vec3::new(-1.0, 1.0, 0.0).normalize();
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(ns)) = m.attribute(Mesh::ATTRIBUTE_NORMAL) {
            let got = Vec3::from_array(ns[1 * w + 1]);
            assert!((got - n).length() < 1e-5, "{got:?}");
        } else {
            panic!("no normals");
        }
        assert!(Grid::from_bytes(&bytes[..8], w, h, 1.0).is_err());
    }

    #[test]
    fn height_at_matches_cell_centres_and_interpolates() {
        let (w, h) = (4usize, 3usize);
        let mut bytes = Vec::new();
        for r in 0..h {
            for c in 0..w {
                bytes.extend_from_slice(&((10 * r + c) as f32).to_le_bytes());
            }
        }
        let g = Grid::from_bytes(&bytes, w, h, 1.0).unwrap();
        for r in 0..h {
            for c in 0..w {
                let p = g.pos(r, c);
                assert!((g.height_at(p.x, -p.z) - p.y).abs() < 1e-5, "cell ({r},{c})");
            }
        }
        // halfway between cell (0,0)=0 and (0,1)=1 on the north row
        assert!((g.height_at(1.0, 2.5) - 0.5).abs() < 1e-5);
        assert!(g.bounding_radius() > (2.0f32 * 2.0 + 1.5 * 1.5).sqrt());
        let s = g.skirt(5.0);
        assert_eq!(s.count_vertices(), 2 * (2 * w + 2 * h - 4));
    }

    /// Every triangle must face up (+y) so Bevy's default back-face culling keeps the terrain visible from above.
    #[test]
    fn all_triangles_face_up() {
        let (w, h) = (5usize, 4usize);
        let mut bytes = Vec::new();
        for r in 0..h {
            for c in 0..w {
                bytes.extend_from_slice(&((c * r) as f32 * 0.1).to_le_bytes()); // gently curved, never vertical
            }
        }
        let m = Grid::from_bytes(&bytes, w, h, 1.0).unwrap().mesh();
        let pos = match m.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
            _ => panic!("no positions"),
        };
        let idx: Vec<u32> = match m.indices() {
            Some(Indices::U32(i)) => i.clone(),
            _ => panic!("no u32 indices"),
        };
        assert_eq!(idx.len(), (w - 1) * (h - 1) * 6);
        for t in idx.chunks_exact(3) {
            let (a, b, c) = (Vec3::from_array(pos[t[0] as usize]), Vec3::from_array(pos[t[1] as usize]), Vec3::from_array(pos[t[2] as usize]));
            let n = (b - a).cross(c - a);
            assert!(n.y > 0.0, "triangle {t:?} faces down: {n:?}");
        }
    }
}
