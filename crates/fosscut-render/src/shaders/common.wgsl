// Shared by every pass: a triangle covering the whole target, and the transfer functions
// (ADR-0008, item 4). Transfer ids match `TransferFunction` in color.rs.

fn full_screen_position(index: u32) -> vec4f {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4f(x, y, 0.0, 1.0);
}

fn decode_transfer(v: vec3f, transfer: u32) -> vec3f {
    if transfer == 0u {
        let high = pow((v + vec3f(0.099)) / 1.099, vec3f(1.0 / 0.45));
        return select(high, v / 4.5, v < vec3f(0.081));
    }
    if transfer == 1u {
        let high = pow((v + vec3f(0.055)) / 1.055, vec3f(2.4));
        return select(high, v / 12.92, v <= vec3f(0.04045));
    }
    return v;
}

fn encode_transfer(l: vec3f, transfer: u32) -> vec3f {
    let c = max(l, vec3f(0.0));
    if transfer == 0u {
        let high = 1.099 * pow(c, vec3f(0.45)) - vec3f(0.099);
        return select(high, 4.5 * c, c < vec3f(0.018));
    }
    if transfer == 1u {
        let high = 1.055 * pow(c, vec3f(1.0 / 2.4)) - vec3f(0.055);
        return select(high, 12.92 * c, c <= vec3f(0.0031308));
    }
    return c;
}
