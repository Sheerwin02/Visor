//! CPU mipmap generation for RGBA8 images (Bevy 0.20 does not generate mips for images built in code).
//! Box filter (2x2 average) in the stored sRGB values: a standard, slightly simplified choice for photographs.
use bevy::image::{Image, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};

/// Number of mip levels for a w x h texture, down to 1 x 1.
pub fn level_count(w: u32, h: u32) -> u32 {
    32 - w.max(h).max(1).leading_zeros()
}

/// All mip levels of an RGBA8 image, level 0 first, concatenated (the layout Bevy expects for one layer).
pub fn build_levels(rgba: &[u8], w: u32, h: u32) -> (Vec<u8>, u32) {
    assert_eq!(rgba.len(), (w * h * 4) as usize, "expected tightly packed RGBA8");
    let levels = level_count(w, h);
    let mut out = rgba.to_vec();
    let (mut cw, mut ch) = (w, h);
    let mut prev = rgba.to_vec();
    for _ in 1..levels {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    let mut n = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((2 * x + dx).min(cw - 1), (2 * y + dy).min(ch - 1));
                        sum += prev[((sy * cw + sx) * 4 + c) as usize] as u32;
                        n += 1;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = ((sum + n / 2) / n) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        cw = nw;
        ch = nh;
    }
    (out, levels)
}

/// Replace an RGBA8 image's data with a full mip chain and use trilinear filtering.
pub fn with_mips(mut img: Image) -> Image {
    let (w, h) = (img.texture_descriptor.size.width, img.texture_descriptor.size.height);
    if let Some(data) = img.data.take() {
        let (all, levels) = build_levels(&data, w, h);
        img.data = Some(all);
        img.texture_descriptor.mip_level_count = levels;
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            // sharp ground at grazing angles in Walk mode (WebGL2: EXT_texture_filter_anisotropic, widely supported)
            anisotropy_clamp: 16,
            ..Default::default()
        });
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_layout() {
        assert_eq!(level_count(1, 1), 1);
        assert_eq!(level_count(4, 2), 3);
        assert_eq!(level_count(1106, 2156), 12);
        // 4 x 2 image: left 2x2 block black, right 2x2 block white
        let mut px = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                px.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let (all, levels) = build_levels(&px, 4, 2);
        assert_eq!(levels, 3);
        assert_eq!(all.len(), (8 + 2 + 1) * 4);
        assert_eq!(&all[32..36], &[0, 0, 0, 255]); // level 1, left pixel
        assert_eq!(&all[36..40], &[255, 255, 255, 255]); // level 1, right pixel
        assert_eq!(all[40], 128); // level 2: average of black and white (127.5 rounded)
    }
}
