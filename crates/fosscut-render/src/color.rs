//! The color pipeline's math (ADR-0008, item 4), shared by the shaders (as uniforms) and by
//! tests (as a CPU reference).
//!
//! Frames are converted from Y'CbCr to R'G'B' with the stream's matrix and range, decoded to
//! linear light with the inverse of their transfer function, composited in linear Rec.709,
//! and encoded with the output transfer function. Decoding with the inverse OETF (rather
//! than a display EOTF) makes an untouched pixel round-trip exactly, which is what an
//! editor needs; display rendering intent belongs to the output transform.
//!
//! Graphics are display-referred (ADR-0041): hex colours, web composition frames and
//! pictures tagged sRGB are decoded with [`GRAPHICS_TRANSFER`], the inverse of the output
//! transfer, so their code values come out of an export unchanged.

use fosscut_media::{ColorInfo, Matrix, Range, Transfer};

use crate::RenderError;

/// A 3x4 affine matrix mapping normalized 8-bit code values `[y, cb, cr, 1]` (each code
/// divided by 255, as the GPU samples them) to non-linear R'G'B'.
pub type YuvMatrix = [[f32; 4]; 3];

/// Luma coefficients (Kr, Kb) of a matrix.
fn coefficients(matrix: Matrix) -> Result<(f64, f64), RenderError> {
    match matrix {
        Matrix::Bt601 => Ok((0.299, 0.114)),
        Matrix::Bt709 => Ok((0.2126, 0.0722)),
        Matrix::Bt2020Ncl => Ok((0.2627, 0.0593)),
        Matrix::Identity | Matrix::Unspecified => Err(RenderError::UnsupportedColor {
            message: format!("{matrix:?} matrix for Y'CbCr frames"),
        }),
    }
}

/// The Y'CbCr to R'G'B' matrix for a stream's matrix and range (ITU-R BT.601, BT.709 and
/// BT.2020 equations; 8-bit limited range is 16-235 for luma and 16-240 for chroma).
pub fn yuv_to_rgb(color: &ColorInfo) -> Result<YuvMatrix, RenderError> {
    let (kr, kb) = coefficients(color.matrix)?;
    let kg = 1.0 - kr - kb;
    // Normalized texture value v = code / 255: Y' = v * y_scale + y_offset, and likewise
    // for the chroma difference signals centred on 128.
    let (y_scale, y_offset, c_scale) = match color.range {
        // Full range: codes map straight to 0..1, chroma differences to -0.5..0.5.
        Range::Full => (1.0, 0.0, 1.0),
        Range::Limited | Range::Unspecified => (255.0 / 219.0, -16.0 / 219.0, 255.0 / 224.0),
    };
    let c_offset = -128.0 / 255.0 * c_scale;
    // R' = Y' + 2(1 - Kr) Cr'
    // G' = Y' - 2 Kb (1 - Kb) / Kg Cb' - 2 Kr (1 - Kr) / Kg Cr'
    // B' = Y' + 2(1 - Kb) Cb'
    let r_cr = 2.0 * (1.0 - kr);
    let g_cb = -2.0 * kb * (1.0 - kb) / kg;
    let g_cr = -2.0 * kr * (1.0 - kr) / kg;
    let b_cb = 2.0 * (1.0 - kb);
    let row = |cb: f64, cr: f64| -> [f32; 4] {
        [
            y_scale as f32,
            (cb * c_scale) as f32,
            (cr * c_scale) as f32,
            (y_offset + cb * c_offset + cr * c_offset) as f32,
        ]
    };
    Ok([row(0.0, r_cr), row(g_cb, g_cr), row(b_cb, 0.0)])
}

/// Transfer functions the pipeline can decode and encode, with their shader ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferFunction {
    /// ITU-R BT.709 (also BT.601 and BT.2020 SDR).
    Bt709 = 0,
    /// IEC 61966-2-1 sRGB.
    Srgb = 1,
    /// Linear light.
    Linear = 2,
}

impl TransferFunction {
    /// The function a stream with this transfer tag is decoded with. Sources tagged sRGB
    /// (web composition frames, pictures with an sRGB chunk, screen recordings) are
    /// graphics and decode with [`GRAPHICS_TRANSFER`] (ADR-0041). HDR transfers need the
    /// HDR pipeline, which does not exist yet.
    pub fn for_source(transfer: Transfer) -> Result<TransferFunction, RenderError> {
        match transfer {
            Transfer::Bt709 | Transfer::Unspecified => Ok(TransferFunction::Bt709),
            Transfer::Srgb => Ok(GRAPHICS_TRANSFER),
            Transfer::Linear => Ok(TransferFunction::Linear),
            Transfer::Pq | Transfer::Hlg => Err(RenderError::UnsupportedColor {
                message: format!(
                    "{transfer:?} (HDR) sources need the HDR pipeline, which is not implemented yet"
                ),
            }),
        }
    }

    /// Non-linear signal to linear light.
    pub fn decode(self, v: f32) -> f32 {
        match self {
            TransferFunction::Bt709 => {
                if v < 0.081 {
                    v / 4.5
                } else {
                    ((v + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
            TransferFunction::Srgb => {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }
            TransferFunction::Linear => v,
        }
    }

    /// Linear light to non-linear signal.
    pub fn encode(self, l: f32) -> f32 {
        match self {
            TransferFunction::Bt709 => {
                if l < 0.018 {
                    4.5 * l
                } else {
                    1.099 * l.powf(0.45) - 0.099
                }
            }
            TransferFunction::Srgb => {
                if l <= 0.003_130_8 {
                    12.92 * l
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }
            TransferFunction::Linear => l,
        }
    }
}

/// The transfer function graphics are decoded with (ADR-0041): the inverse of the output
/// transfer, so colours given as code values (hex colours, SVG drawings, web composition
/// frames, pictures tagged sRGB) come out of an output with the same code values. Every
/// output FossCut writes today (exports, frame grabs, the preview) is SDR encoded with
/// BT.709. HDR outputs will need a reference white for graphics instead.
pub const GRAPHICS_TRANSFER: TransferFunction = TransferFunction::Bt709;

/// A graphics code value from 0 to 1, such as a channel of a hex colour or a pixel of an
/// SVG drawing, in linear light (ADR-0041).
pub fn graphics_to_linear(v: f32) -> f32 {
    GRAPHICS_TRANSFER.decode(v)
}

/// The graphics code value from 0 to 1 that [`graphics_to_linear`] turns into linear
/// light `l`.
pub fn graphics_from_linear(l: f32) -> f32 {
    GRAPHICS_TRANSFER.encode(l)
}

/// A `#RRGGBB` or `#RRGGBBAA` colour as linear-light RGB and straight alpha (ADR-0041): its
/// channels are the code values an export keeps. Alpha is 1 without `AA`; a channel that
/// does not parse reads as full.
pub fn hex_to_linear(hex: &str) -> [f32; 4] {
    let digits = hex.trim_start_matches('#');
    let channel = |i: usize| {
        digits
            .get(i..i + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .map_or(1.0, |v| f32::from(v) / 255.0)
    };
    let alpha = if digits.len() >= 8 { channel(6) } else { 1.0 };
    [
        graphics_to_linear(channel(0)),
        graphics_to_linear(channel(2)),
        graphics_to_linear(channel(4)),
        alpha,
    ]
}

/// Converts one 8-bit Y'CbCr sample to linear RGB on the CPU, exactly as the shaders do.
pub fn reference_yuv_to_linear(
    color: &ColorInfo,
    y: u8,
    cb: u8,
    cr: u8,
) -> Result<[f32; 3], RenderError> {
    let matrix = yuv_to_rgb(color)?;
    let transfer = TransferFunction::for_source(color.transfer)?;
    let input = [
        f32::from(y) / 255.0,
        f32::from(cb) / 255.0,
        f32::from(cr) / 255.0,
        1.0,
    ];
    let mut out = [0.0; 3];
    for (channel, row) in out.iter_mut().zip(matrix) {
        let value: f32 = row.iter().zip(input).map(|(m, v)| m * v).sum();
        *channel = transfer.decode(value.clamp(0.0, 1.0));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fosscut_media::Primaries;

    fn bt709(range: Range) -> ColorInfo {
        ColorInfo {
            matrix: Matrix::Bt709,
            range,
            primaries: Primaries::Bt709,
            transfer: Transfer::Bt709,
        }
    }

    fn encoded(color: &ColorInfo, y: u8, cb: u8, cr: u8) -> [f32; 3] {
        reference_yuv_to_linear(color, y, cb, cr)
            .unwrap()
            .map(|l| TransferFunction::Bt709.encode(l))
    }

    fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
    }

    #[test]
    fn limited_range_black_white_and_75_percent_bars() {
        let color = bt709(Range::Limited);
        assert!(close(encoded(&color, 16, 128, 128), [0.0; 3], 1e-6));
        assert!(close(encoded(&color, 235, 128, 128), [1.0; 3], 1e-6));
        // 75% bars in BT.709 limited range (SMPTE RP 219 code values).
        let tolerance = 1.5 / 255.0;
        assert!(close(
            encoded(&color, 51, 109, 212),
            [0.75, 0.0, 0.0],
            tolerance
        ));
        assert!(close(
            encoded(&color, 133, 63, 52),
            [0.0, 0.75, 0.0],
            tolerance
        ));
        assert!(close(
            encoded(&color, 28, 212, 120),
            [0.0, 0.0, 0.75],
            tolerance
        ));
        assert!(close(
            encoded(&color, 168, 44, 136),
            [0.75, 0.75, 0.0],
            tolerance
        ));
    }

    #[test]
    fn full_range_and_bt601_use_their_own_equations() {
        let full = bt709(Range::Full);
        assert!(close(encoded(&full, 0, 128, 128), [0.0; 3], 1e-6));
        assert!(close(encoded(&full, 255, 128, 128), [1.0; 3], 1e-6));
        let sd = ColorInfo {
            matrix: Matrix::Bt601,
            ..bt709(Range::Limited)
        };
        // BT.601 limited range pure red: Y 81, Cb 90, Cr 240.
        assert!(close(
            encoded(&sd, 81, 90, 240),
            [1.0, 0.0, 0.0],
            2.0 / 255.0
        ));
        let wide = ColorInfo {
            matrix: Matrix::Bt2020Ncl,
            ..bt709(Range::Limited)
        };
        assert!(close(encoded(&wide, 235, 128, 128), [1.0; 3], 1e-6));
    }

    #[test]
    fn transfers_round_trip() {
        for transfer in [
            TransferFunction::Bt709,
            TransferFunction::Srgb,
            TransferFunction::Linear,
        ] {
            for i in 0..=100 {
                let v = i as f32 / 100.0;
                let back = transfer.encode(transfer.decode(v));
                assert!((back - v).abs() < 1e-5, "{transfer:?} at {v}: {back}");
            }
        }
        assert!((TransferFunction::Srgb.decode(0.5) - 0.214_041).abs() < 1e-5);
        assert!((TransferFunction::Bt709.decode(0.5) - 0.259_589).abs() < 1e-5);
    }

    #[test]
    fn graphics_keep_their_code_values_in_the_output() {
        let output = TransferFunction::Bt709;
        for code in 0..=255u8 {
            let v = f32::from(code) / 255.0;
            let out = output.encode(graphics_to_linear(v)) * 255.0;
            assert!((out - f32::from(code)).abs() < 0.01, "{code}: {out}");
            assert!((graphics_from_linear(graphics_to_linear(v)) - v).abs() < 1e-5);
        }
        // A dark brand colour: decoded as sRGB it came out as (30, 43, 72).
        let [r, g, b, a] = hex_to_linear("#2E3B57");
        let codes = [r, g, b].map(|l| (output.encode(l) * 255.0).round() as u8);
        assert_eq!((codes, a), ([46, 59, 87], 1.0));
        // Sources tagged sRGB are graphics too.
        assert_eq!(
            TransferFunction::for_source(Transfer::Srgb).unwrap(),
            GRAPHICS_TRANSFER
        );
    }

    #[test]
    fn hex_colours_decode_to_linear_light_with_alpha() {
        assert_eq!(hex_to_linear("#FFFFFF"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(hex_to_linear("#000000"), [0.0, 0.0, 0.0, 1.0]);
        let mid = hex_to_linear("#808080");
        // ((128 / 255 + 0.099) / 1.099) ^ (1 / 0.45), not the 0.2158 of the sRGB curve.
        assert!((mid[0] - 0.261_482).abs() < 1e-5, "{mid:?}");
        let translucent = hex_to_linear("#FF000080");
        assert_eq!(translucent[..3], [1.0, 0.0, 0.0]);
        assert!((translucent[3] - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(hex_to_linear("bad")[3], 1.0);
        assert_eq!(hex_to_linear("#12")[1..], [1.0, 1.0, 1.0]);
    }

    #[test]
    fn unsupported_encodings_are_typed_errors() {
        let identity = ColorInfo {
            matrix: Matrix::Identity,
            ..bt709(Range::Full)
        };
        assert_eq!(
            yuv_to_rgb(&identity).unwrap_err().code(),
            "fosscut.render.unsupported_color"
        );
        for hdr in [Transfer::Pq, Transfer::Hlg] {
            assert_eq!(
                TransferFunction::for_source(hdr).unwrap_err().code(),
                "fosscut.render.unsupported_color"
            );
        }
        assert_eq!(
            TransferFunction::for_source(Transfer::Unspecified).unwrap(),
            TransferFunction::Bt709
        );
    }
}
