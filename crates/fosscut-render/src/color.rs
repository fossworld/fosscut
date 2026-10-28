//! The color pipeline's math (ADR-0008, item 4), shared by the shaders (as uniforms) and by
//! tests (as a CPU reference).
//!
//! Frames are converted from Y'CbCr to R'G'B' with the stream's matrix and range, decoded to
//! linear light with the inverse of their transfer function, composited in linear Rec.709,
//! and encoded with the output transfer function. Decoding with the inverse OETF (rather
//! than a display EOTF) makes an untouched pixel round-trip exactly, which is what an
//! editor needs; display rendering intent belongs to the output transform.

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
    /// The function for a stream's transfer tag. HDR transfers need the HDR pipeline,
    /// which does not exist yet.
    pub fn for_source(transfer: Transfer) -> Result<TransferFunction, RenderError> {
        match transfer {
            Transfer::Bt709 | Transfer::Unspecified => Ok(TransferFunction::Bt709),
            Transfer::Srgb => Ok(TransferFunction::Srgb),
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
