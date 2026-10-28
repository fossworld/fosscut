//! 3D lookup tables in the Adobe/Resolve `.cube` format (ADR-0011: "LUT `.cube` import").

use crate::RenderError;

/// A 3D LUT: `size`³ RGB entries with red varying fastest, inputs and outputs in 0..1.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    /// Entries per axis.
    pub size: u32,
    /// `size³` RGB triples, red fastest, then green, then blue.
    pub data: Vec<[f32; 3]>,
}

/// Largest supported size per axis; common LUTs are 17, 33 or 65.
const MAX_SIZE: u32 = 256;

impl Lut3d {
    /// The LUT that leaves colors unchanged.
    pub fn identity(size: u32) -> Lut3d {
        let size = size.clamp(2, MAX_SIZE);
        let step = 1.0 / (size - 1) as f32;
        let mut data = Vec::with_capacity((size * size * size) as usize);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push([r as f32 * step, g as f32 * step, b as f32 * step]);
                }
            }
        }
        Lut3d { size, data }
    }

    /// Parses `.cube` text. Supports `LUT_3D_SIZE`, `TITLE`, comments, and
    /// `DOMAIN_MIN`/`DOMAIN_MAX` of 0 and 1 only.
    pub fn parse_cube(text: &str) -> Result<Lut3d, RenderError> {
        let invalid = |line: usize, message: &str| RenderError::InvalidLut {
            message: format!("line {line}: {message}"),
        };
        let mut size: Option<u32> = None;
        let mut data = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let number = index + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split_whitespace();
            let first = fields.next().unwrap_or("");
            match first {
                "TITLE" => {}
                "LUT_1D_SIZE" => return Err(invalid(number, "1D LUTs are not supported")),
                "LUT_3D_SIZE" => {
                    let value: u32 = fields
                        .next()
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| invalid(number, "LUT_3D_SIZE needs a number"))?;
                    if !(2..=MAX_SIZE).contains(&value) {
                        return Err(invalid(number, "LUT_3D_SIZE must be between 2 and 256"));
                    }
                    size = Some(value);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let expected = if first == "DOMAIN_MIN" { 0.0 } else { 1.0 };
                    let values: Vec<f32> = fields.filter_map(|v| v.parse().ok()).collect();
                    if values.len() != 3 || values.iter().any(|v| *v != expected) {
                        return Err(invalid(number, "only a 0 to 1 domain is supported"));
                    }
                }
                _ => {
                    let values: Vec<f32> = line
                        .split_whitespace()
                        .map(|v| v.parse::<f32>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| invalid(number, "expected three numbers"))?;
                    let [r, g, b] = values[..] else {
                        return Err(invalid(number, "expected three numbers"));
                    };
                    data.push([r, g, b]);
                }
            }
        }
        let size = size.ok_or_else(|| RenderError::InvalidLut {
            message: "missing LUT_3D_SIZE".to_string(),
        })?;
        let expected = (size * size * size) as usize;
        if data.len() != expected {
            return Err(RenderError::InvalidLut {
                message: format!("expected {expected} entries, found {}", data.len()),
            });
        }
        Ok(Lut3d { size, data })
    }

    /// Looks up a color with trilinear interpolation, as the GPU's filtered 3D texture does.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size as usize;
        let max = (n - 1) as f32;
        let position = rgb.map(|v| v.clamp(0.0, 1.0) * max);
        let low = position.map(|p| (p.floor() as usize).min(n - 2));
        let fraction = [0, 1, 2].map(|i| position[i] - low[i] as f32);
        let at = |r: usize, g: usize, b: usize| self.data[r + n * (g + n * b)];
        let mut out = [0.0; 3];
        for corner in 0..8 {
            let (dr, dg, db) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let weight = [dr, dg, db]
                .iter()
                .zip(fraction)
                .map(|(d, f)| if *d == 1 { f } else { 1.0 - f })
                .product::<f32>();
            let value = at(low[0] + dr, low[1] + dg, low[2] + db);
            for c in 0..3 {
                out[c] += weight * value[c];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_colors_to_themselves() {
        let lut = Lut3d::identity(17);
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.25, 0.5, 0.8],
            [0.3, 0.71, 0.05],
        ] {
            let out = lut.apply(rgb);
            for c in 0..3 {
                assert!((out[c] - rgb[c]).abs() < 1e-5, "{rgb:?} -> {out:?}");
            }
        }
        assert_eq!(Lut3d::identity(1).size, 2);
    }

    #[test]
    fn cube_files_parse_with_comments_title_and_domain() {
        let text = "# made by hand\nTITLE \"Invert\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
                    1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
        let lut = Lut3d::parse_cube(text).unwrap();
        assert_eq!(lut.size, 2);
        let out = lut.apply([0.25, 0.5, 1.0]);
        assert!((out[0] - 0.75).abs() < 1e-6 && (out[1] - 0.5).abs() < 1e-6 && out[2].abs() < 1e-6);
    }

    #[test]
    fn malformed_cube_files_are_rejected_with_a_reason() {
        for (text, reason) in [
            ("0 0 0\n", "missing LUT_3D_SIZE"),
            ("LUT_3D_SIZE 2\n0 0 0\n", "expected 8 entries"),
            ("LUT_3D_SIZE 2\n0 0\n", "expected three numbers"),
            ("LUT_3D_SIZE 2\n0 zero 0\n", "expected three numbers"),
            ("LUT_3D_SIZE 1\n", "between 2 and 256"),
            ("LUT_3D_SIZE\n", "needs a number"),
            ("LUT_1D_SIZE 16\n", "1D LUTs"),
            ("LUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2\n", "0 to 1 domain"),
        ] {
            let error = Lut3d::parse_cube(text).unwrap_err();
            assert_eq!(error.code(), "fosscut.render.invalid_lut");
            assert!(error.to_string().contains(reason), "{text:?}: {error}");
        }
    }
}
