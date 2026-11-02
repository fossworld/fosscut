// FossCut's scene kit for Three.js compositions (ADR-0042), written for FossCut's library and
// free to use, also commercially. A Stage renders a scene for one composition time:
//
// - motion blur: several sub-frames spread over the shutter interval are rendered and
//   averaged in linear light (the sub-frame nearest the frame's time is rendered last, so
//   its depth feeds the depth of field);
// - depth of field: a gather blur whose radius grows with the distance from the focus
//   (thin-lens circle of confusion), which keeps sharp edges in front from bleeding;
// - bloom: bright parts above a threshold are blurred through a chain of half-size
//   levels and added back;
// - output: tone mapping, sRGB encoding and premultiplied alpha, so a transparent
//   background stays transparent and glow over it keeps its light.
//
// Every frame is a function of its time only: nothing is carried from one frame to the
// next, and randomness comes from seeded generators.

import * as THREE from 'three';

/** Clamps `value` to `low`..`high`. */
export const clamp = (value, low = 0, high = 1) => Math.min(high, Math.max(low, value));

/** Linear interpolation from `a` to `b` by `t`. */
export const mix = (a, b, t) => a + (b - a) * t;

/** How far `t` is through the span starting at `start` lasting `duration`, from 0 to 1. */
export const progress = (t, start, duration) => {
  if (duration <= 0) {
    return t >= start ? 1 : 0;
  }
  return clamp((t - start) / duration);
};

/** Easing curves from 0 to 1. */
export const ease = {
  linear: (x) => x,
  inCubic: (x) => x * x * x,
  outCubic: (x) => 1 - (1 - x) ** 3,
  inOutCubic: (x) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2),
  outQuint: (x) => 1 - (1 - x) ** 5,
  inOutQuint: (x) => (x < 0.5 ? 16 * x ** 5 : 1 - (-2 * x + 2) ** 5 / 2),
  inOutSine: (x) => -(Math.cos(Math.PI * x) - 1) / 2,
  outExpo: (x) => (x >= 1 ? 1 : 1 - 2 ** (-10 * x)),
  inOutExpo: (x) => {
    if (x <= 0 || x >= 1) {
      return x <= 0 ? 0 : 1;
    }
    return x < 0.5 ? 2 ** (20 * x - 10) / 2 : (2 - 2 ** (-20 * x + 10)) / 2;
  },
  outBack: (x, overshoot = 1.4) => 1 + (overshoot + 1) * (x - 1) ** 3 + overshoot * (x - 1) ** 2,
};

/**
 * A damped spring settling from 0 to 1 over `t` seconds after it starts: `frequency`
 * oscillations per second, `damping` from 0 (rings forever) to 1 (no overshoot).
 */
export const spring = (t, frequency = 2, damping = 0.4) => {
  if (t <= 0) {
    return 0;
  }
  const omega = 2 * Math.PI * frequency;
  const zeta = clamp(damping, 0.01, 0.999);
  const damped = omega * Math.sqrt(1 - zeta * zeta);
  const decay = Math.exp(-zeta * omega * t);
  return 1 - decay * (Math.cos(damped * t) + (zeta * omega / damped) * Math.sin(damped * t));
};

/** A generator of numbers from 0 to 1 that gives the same sequence for the same seed. */
export const random = (seed) => {
  let state = (Math.floor(Number(seed) || 0) ^ 0x5f3759df) | 0;
  return () => {
    state = (state + 0x9e3779b9) | 0;
    let z = state;
    z = Math.imul(z ^ (z >>> 16), 0x85ebca6b);
    z = Math.imul(z ^ (z >>> 13), 0xc2b2ae35);
    z ^= z >>> 16;
    return (z >>> 0) / 4294967296;
  };
};

const isObject = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

/** `base` with the values of `over` in place, groups merged field by field. */
export const merge = (base, over) => {
  const out = { ...base };
  if (!isObject(over)) {
    return out;
  }
  for (const [key, value] of Object.entries(over)) {
    out[key] = isObject(value) && isObject(base[key]) ? merge(base[key], value) : value;
  }
  return out;
};

/**
 * The composition's parameters: `params.json` from its own folder, which FossCut writes
 * when it adds the composition, over `defaults`, which also apply when the file is missing.
 */
export const loadParams = async (defaults) => {
  try {
    const response = await fetch('params.json');
    if (response.ok) {
      return merge(defaults, await response.json());
    }
  } catch (error) {
    // No params.json: the defaults.
  }
  return merge(defaults, {});
};

/** The composition's frame rate, from its `fosscut:composition` meta tag (30 when absent). */
export const frameRate = () => {
  const meta = document.querySelector('meta[name="fosscut:composition"]');
  const found = /fps\s*=\s*([0-9.]+)(?:\s*\/\s*([0-9.]+))?/i.exec(meta ? meta.content : '');
  if (!found) {
    return 30;
  }
  const rate = Number(found[1]) / (found[2] ? Number(found[2]) : 1);
  return rate > 0 ? rate : 30;
};

/** A colour from `#RRGGBB` text, in linear light. */
export const color = (hex, fallback = '#ffffff') => new THREE.Color(typeof hex === 'string' ? hex : fallback);

/** `hex` brightened `times` past white, for emissive parts that should glow. */
export const glow = (hex, times) => color(hex).multiplyScalar(times);

/** Loads a texture from the composition's folder; resolves to `null` when it fails. */
export const loadTexture = (src) => new Promise((resolve) => {
  if (!src) {
    resolve(null);
    return;
  }
  new THREE.TextureLoader().load(src, (texture) => {
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.anisotropy = 8;
    resolve(texture);
  }, undefined, () => resolve(null));
});

/** A texture of `size` pixels whose pixels are `paint(u, v)` as [r, g, b, a] from 0 to 1. */
export const paintTexture = (size, paint, srgb = false) => {
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const [r, g, b, a] = paint((x + 0.5) / size, (y + 0.5) / size);
      const i = (y * size + x) * 4;
      data[i] = Math.round(clamp(r) * 255);
      data[i + 1] = Math.round(clamp(g) * 255);
      data[i + 2] = Math.round(clamp(b) * 255);
      data[i + 3] = Math.round(clamp(a) * 255);
    }
  }
  const texture = new THREE.DataTexture(data, size, size, THREE.RGBAFormat);
  texture.colorSpace = srgb ? THREE.SRGBColorSpace : THREE.NoColorSpace;
  texture.magFilter = THREE.LinearFilter;
  texture.minFilter = THREE.LinearMipmapLinearFilter;
  texture.generateMipmaps = true;
  texture.wrapS = THREE.RepeatWrapping;
  texture.wrapT = THREE.RepeatWrapping;
  texture.needsUpdate = true;
  return texture;
};

/** A soft round shadow or glow: opaque in the middle, fading to nothing at the edge. */
export const softDisc = (size = 128, softness = 1) => paintTexture(size, (u, v) => {
  const d = Math.min(1, Math.hypot(u - 0.5, v - 0.5) * 2);
  const a = (1 - d * d) ** (2 + 2 * softness) * (1 - d);
  return [1, 1, 1, a];
});

/**
 * A studio for image-based lighting, made in code: a dark room with a large soft box above,
 * two strip lights at the sides and a warm and a cool bounce. Gives the scene's
 * environment map.
 */
export const studio = (renderer, { key = 1, tint = '#ffffff', rim = '#ffffff' } = {}) => {
  const room = new THREE.Scene();
  const walls = new THREE.Mesh(
    new THREE.BoxGeometry(30, 18, 30),
    new THREE.MeshBasicMaterial({ color: new THREE.Color(0.035, 0.037, 0.042), side: THREE.BackSide }),
  );
  room.add(walls);
  const panel = (w, h, hex, strength, position, target) => {
    const mesh = new THREE.Mesh(
      new THREE.PlaneGeometry(w, h),
      new THREE.MeshBasicMaterial({ color: color(hex).multiplyScalar(strength), side: THREE.DoubleSide }),
    );
    mesh.position.set(...position);
    mesh.lookAt(new THREE.Vector3(...target));
    room.add(mesh);
  };
  panel(10, 6, tint, 6 * key, [0, 8.5, 2], [0, 0, 0]);
  panel(2.2, 12, '#ffffff', 5 * key, [-12, 2, 3], [0, 0, 0]);
  panel(2.2, 12, '#ffffff', 3.5 * key, [12, 2, -2], [0, 0, 0]);
  panel(8, 3, rim, 4 * key, [0, 3, -13], [0, 0, 0]);
  panel(14, 4, '#ffd9b0', 0.6 * key, [0, -6, 8], [0, 0, 0]);
  panel(6, 6, '#b8d4ff', 0.8 * key, [10, -2, 10], [0, 0, 0]);
  const generator = new THREE.PMREMGenerator(renderer);
  const target = generator.fromScene(room, 0.035);
  generator.dispose();
  walls.geometry.dispose();
  return target.texture;
};

// Full-screen passes.

const VERTEX = /* glsl */ `
varying vec2 vUv;
void main() {
  vUv = position.xy * 0.5 + 0.5;
  gl_Position = vec4(position.xy, 0.0, 1.0);
}`;

const ACCUMULATE = /* glsl */ `
uniform sampler2D tSource;
uniform float weight;
varying vec2 vUv;
void main() {
  gl_FragColor = texture2D(tSource, vUv) * weight;
}`;

const DEPTH_OF_FIELD = /* glsl */ `
uniform sampler2D tColor;
uniform sampler2D tDepth;
uniform vec2 texel;
uniform float near;
uniform float far;
uniform float focus;
uniform float aperture;
uniform float maxRadius;
varying vec2 vUv;
const int SAMPLES = 64;
float distanceAt(vec2 uv) {
  float ndc = texture2D(tDepth, uv).r * 2.0 - 1.0;
  return 2.0 * near * far / (far + near - ndc * (far - near));
}
float radiusAt(float z) {
  return maxRadius * clamp(2.0 * aperture * abs(z - focus) / max(z, 1e-4), 0.0, 1.0);
}
void main() {
  vec4 centre = texture2D(tColor, vUv);
  float z = distanceAt(vUv);
  float radius = radiusAt(z);
  if (radius < 0.5) {
    gl_FragColor = centre;
    return;
  }
  vec4 sum = centre;
  float total = 1.0;
  for (int i = 0; i < SAMPLES; i++) {
    float f = (float(i) + 0.5) / float(SAMPLES);
    float r = sqrt(f) * radius;
    float a = float(i) * 2.39996323;
    vec2 uv = vUv + vec2(cos(a), sin(a)) * r * texel;
    float sz = distanceAt(uv);
    // A sharper sample in front of this pixel does not spread over it beyond its own blur.
    float reach = sz < z ? min(radiusAt(sz), radius) : radius;
    float w = clamp(reach - r + 1.0, 0.0, 1.0);
    sum += texture2D(tColor, uv) * w;
    total += w;
  }
  gl_FragColor = sum / total;
}`;

const BRIGHT = /* glsl */ `
uniform sampler2D tSource;
uniform vec2 texel;
uniform float threshold;
varying vec2 vUv;
void main() {
  vec4 c = 0.25 * (texture2D(tSource, vUv + texel * vec2(-0.5, -0.5))
    + texture2D(tSource, vUv + texel * vec2(0.5, -0.5))
    + texture2D(tSource, vUv + texel * vec2(-0.5, 0.5))
    + texture2D(tSource, vUv + texel * vec2(0.5, 0.5)));
  float brightness = max(c.r, max(c.g, c.b));
  float knee = threshold * 0.5 + 1e-4;
  float soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
  soft = soft * soft / (4.0 * knee);
  float keep = max(soft, brightness - threshold) / max(brightness, 1e-4);
  gl_FragColor = vec4(c.rgb * keep, 1.0);
}`;

const DOWN = /* glsl */ `
uniform sampler2D tSource;
uniform vec2 texel;
varying vec2 vUv;
void main() {
  vec3 c = texture2D(tSource, vUv).rgb * 4.0;
  c += texture2D(tSource, vUv + texel * vec2(-1.0, -1.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(1.0, -1.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(-1.0, 1.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(1.0, 1.0)).rgb;
  gl_FragColor = vec4(c / 8.0, 1.0);
}`;

const UP = /* glsl */ `
uniform sampler2D tSource;
uniform sampler2D tLevel;
uniform vec2 texel;
uniform float spread;
varying vec2 vUv;
void main() {
  vec3 c = vec3(0.0);
  c += texture2D(tSource, vUv + texel * vec2(-2.0, 0.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(2.0, 0.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(0.0, -2.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(0.0, 2.0)).rgb;
  c += texture2D(tSource, vUv + texel * vec2(-1.0, -1.0)).rgb * 2.0;
  c += texture2D(tSource, vUv + texel * vec2(1.0, -1.0)).rgb * 2.0;
  c += texture2D(tSource, vUv + texel * vec2(-1.0, 1.0)).rgb * 2.0;
  c += texture2D(tSource, vUv + texel * vec2(1.0, 1.0)).rgb * 2.0;
  gl_FragColor = vec4(texture2D(tLevel, vUv).rgb + c / 12.0 * spread, 1.0);
}`;

const OUTPUT = /* glsl */ `
#include <tonemapping_pars_fragment>
uniform sampler2D tColor;
uniform sampler2D tBloom;
uniform float bloom;
uniform float exposure;
uniform int mapping;
varying vec2 vUv;
void main() {
  vec4 c = texture2D(tColor, vUv);
  vec3 light = (c.rgb + texture2D(tBloom, vUv).rgb * bloom) * exposure;
  vec3 mapped = mapping == 1 ? AgXToneMapping(light)
    : mapping == 2 ? NeutralToneMapping(light)
    : ACESFilmicToneMapping(light);
  mapped = clamp(mapped, 0.0, 1.0);
  // Light over a transparent background makes its own coverage.
  float alpha = clamp(max(c.a, max(mapped.r, max(mapped.g, mapped.b))), 0.0, 1.0);
  vec3 straight = alpha > 0.0 ? clamp(mapped / alpha, 0.0, 1.0) : vec3(0.0);
  gl_FragColor = vec4(sRGBTransferOETF(vec4(straight, 1.0)).rgb * alpha, alpha);
}`;

const pass = (fragmentShader, uniforms, blending = THREE.NoBlending) => {
  const material = new THREE.ShaderMaterial({
    vertexShader: VERTEX,
    fragmentShader,
    uniforms,
    depthTest: false,
    depthWrite: false,
    blending,
    toneMapped: false,
  });
  if (blending === THREE.CustomBlending) {
    material.blendEquation = THREE.AddEquation;
    material.blendSrc = THREE.OneFactor;
    material.blendDst = THREE.OneFactor;
    material.blendSrcAlpha = THREE.OneFactor;
    material.blendDstAlpha = THREE.OneFactor;
  }
  return material;
};

const target = (width, height, options = {}) => new THREE.WebGLRenderTarget(
  Math.max(1, Math.round(width)),
  Math.max(1, Math.round(height)),
  {
    type: THREE.HalfFloatType,
    format: THREE.RGBAFormat,
    minFilter: THREE.LinearFilter,
    magFilter: THREE.LinearFilter,
    wrapS: THREE.ClampToEdgeWrapping,
    wrapT: THREE.ClampToEdgeWrapping,
    depthBuffer: false,
    ...options,
  },
);

const MAPPINGS = { aces: 0, agx: 1, neutral: 2 };

/**
 * A scene, its camera and the passes that finish its frames. `params` are the common scene
 * parameters (`transparent`, `background`, `exposure`, `tone_mapping`, `bloom`,
 * `depth_of_field`, `motion_blur`).
 */
export class Stage {
  constructor(params, { fov = 35, near = 0.1, far = 200 } = {}) {
    this.params = params;
    this.width = Math.max(1, window.innerWidth);
    this.height = Math.max(1, window.innerHeight);
    this.fps = frameRate();
    const canvas = document.createElement('canvas');
    canvas.width = this.width;
    canvas.height = this.height;
    Object.assign(canvas.style, {
      position: 'absolute', left: '0', top: '0', width: `${this.width}px`, height: `${this.height}px`,
    });
    const context = canvas.getContext('webgl2', {
      alpha: true,
      antialias: false,
      depth: true,
      premultipliedAlpha: true,
      preserveDrawingBuffer: true,
      powerPreference: 'high-performance',
    });
    if (!context) {
      throw new Error('WebGL 2 is not available: this composition needs <meta name="fosscut:renderer" content="gpu"> and a browser that draws on the GPU');
    }
    document.body.appendChild(canvas);
    this.canvas = canvas;
    const renderer = new THREE.WebGLRenderer({ canvas, context });
    renderer.setPixelRatio(1);
    renderer.setSize(this.width, this.height, false);
    renderer.autoClear = false;
    renderer.toneMapping = THREE.NoToneMapping;
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;
    this.renderer = renderer;
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(fov, this.width / this.height, near, far);
    // The point the depth of field keeps sharp; templates move it with their subject.
    this.focus = new THREE.Vector3();
    this.background = color(params.background, '#000000');

    const { width, height } = this;
    this.sceneTarget = target(width, height, {
      samples: 4,
      depthBuffer: true,
      depthTexture: new THREE.DepthTexture(width, height, THREE.FloatType),
    });
    this.accumTarget = target(width, height);
    this.focusTarget = target(width, height);
    this.levels = [];
    let w = width / 2;
    let h = height / 2;
    for (let i = 0; i < 6 && w >= 2 && h >= 2; i += 1) {
      this.levels.push({ down: target(w, h), up: target(w, h) });
      w /= 2;
      h /= 2;
    }
    this.black = target(1, 1);

    this.quadCamera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
    const triangle = new THREE.BufferGeometry();
    triangle.setAttribute('position', new THREE.Float32BufferAttribute([-1, -1, 0, 3, -1, 0, -1, 3, 0], 3));
    this.quad = new THREE.Mesh(triangle);
    this.quad.frustumCulled = false;
    this.passes = {
      accumulate: pass(ACCUMULATE, { tSource: { value: null }, weight: { value: 1 } }, THREE.CustomBlending),
      focus: pass(DEPTH_OF_FIELD, {
        tColor: { value: null },
        tDepth: { value: null },
        texel: { value: new THREE.Vector2(1 / width, 1 / height) },
        near: { value: near },
        far: { value: far },
        focus: { value: 10 },
        aperture: { value: 0 },
        maxRadius: { value: 0 },
      }),
      bright: pass(BRIGHT, { tSource: { value: null }, texel: { value: new THREE.Vector2() }, threshold: { value: 1 } }),
      down: pass(DOWN, { tSource: { value: null }, texel: { value: new THREE.Vector2() } }),
      up: pass(UP, {
        tSource: { value: null },
        tLevel: { value: null },
        texel: { value: new THREE.Vector2() },
        spread: { value: 1 },
      }),
      output: pass(OUTPUT, {
        tColor: { value: null },
        tBloom: { value: null },
        bloom: { value: 0 },
        exposure: { value: 1 },
        mapping: { value: 0 },
      }),
    };
  }

  /** Draws pass `material` into `into` (the canvas when null), clearing it first. */
  draw(material, into, clear = true) {
    const { renderer } = this;
    this.quad.material = material;
    renderer.setRenderTarget(into);
    if (clear) {
      renderer.setClearColor(0x000000, 0);
      renderer.clear(true, false, false);
    }
    renderer.render(this.quad, this.quadCamera);
  }

  /**
   * Renders the frame at time `t` seconds: `update(time)` poses the scene for each
   * sub-frame of the shutter, and is called with `t` itself last.
   */
  render(t, update) {
    const { renderer, params } = this;
    const blur = params.motion_blur || {};
    const samples = Math.max(1, Math.round(blur.samples || 1));
    const shutter = clamp(Number(blur.shutter) || 0);
    const offsets = [];
    for (let i = 0; i < samples; i += 1) {
      offsets.push(samples === 1 ? 0 : ((i + 0.5) / samples - 0.5) * (shutter / this.fps));
    }
    // The sub-frame nearest the frame's time goes last: its depth feeds the focus pass.
    offsets.sort((a, b) => Math.abs(b) - Math.abs(a) || a - b);
    const transparent = Boolean(params.transparent);
    for (let i = 0; i < offsets.length; i += 1) {
      update(t + offsets[i]);
      renderer.setRenderTarget(this.sceneTarget);
      renderer.setClearColor(this.background, transparent ? 0 : 1);
      renderer.clear(true, true, true);
      renderer.render(this.scene, this.camera);
      if (samples > 1) {
        this.passes.accumulate.uniforms.tSource.value = this.sceneTarget.texture;
        this.passes.accumulate.uniforms.weight.value = 1 / samples;
        this.draw(this.passes.accumulate, this.accumTarget, i === 0);
      }
    }
    if (offsets[offsets.length - 1] !== 0) {
      update(t);
    }
    let image = samples > 1 ? this.accumTarget.texture : this.sceneTarget.texture;

    const field = params.depth_of_field || {};
    const aperture = Number(field.aperture) || 0;
    const maxRadius = (Number(field.max_blur) || 0) * this.height;
    if (aperture > 0 && maxRadius >= 0.5) {
      const u = this.passes.focus.uniforms;
      u.tColor.value = image;
      u.tDepth.value = this.sceneTarget.depthTexture;
      u.near.value = this.camera.near;
      u.far.value = this.camera.far;
      const view = this.focus.clone().applyMatrix4(this.camera.matrixWorldInverse);
      u.focus.value = Math.max(this.camera.near, -view.z);
      u.aperture.value = aperture;
      u.maxRadius.value = maxRadius;
      this.draw(this.passes.focus, this.focusTarget);
      image = this.focusTarget.texture;
    }

    const glowParams = params.bloom || {};
    const strength = Number(glowParams.strength) || 0;
    let bloomTexture = this.black.texture;
    if (strength > 0 && this.levels.length > 0) {
      const radius = clamp(Number(glowParams.radius) || 0);
      const { bright, down, up } = this.passes;
      bright.uniforms.tSource.value = image;
      bright.uniforms.texel.value.set(1 / this.width, 1 / this.height);
      bright.uniforms.threshold.value = Math.max(0, Number(glowParams.threshold) || 0);
      this.draw(bright, this.levels[0].down);
      for (let i = 1; i < this.levels.length; i += 1) {
        const from = this.levels[i - 1].down;
        down.uniforms.tSource.value = from.texture;
        down.uniforms.texel.value.set(1 / from.width, 1 / from.height);
        this.draw(down, this.levels[i].down);
      }
      let source = this.levels[this.levels.length - 1].down;
      for (let i = this.levels.length - 2; i >= 0; i -= 1) {
        up.uniforms.tSource.value = source.texture;
        up.uniforms.tLevel.value = this.levels[i].down.texture;
        up.uniforms.texel.value.set(0.5 / source.width, 0.5 / source.height);
        // Wider levels count more as the radius grows.
        up.uniforms.spread.value = mix(0.35, 1.25, radius);
        this.draw(up, this.levels[i].up);
        source = this.levels[i].up;
      }
      bloomTexture = source.texture;
    }

    const output = this.passes.output;
    output.uniforms.tColor.value = image;
    output.uniforms.tBloom.value = bloomTexture;
    output.uniforms.bloom.value = strength / Math.max(1, this.levels.length * 0.5);
    output.uniforms.exposure.value = Number(params.exposure) || 1;
    output.uniforms.mapping.value = MAPPINGS[params.tone_mapping] ?? 0;
    this.draw(output, null);
    renderer.setRenderTarget(null);
  }
}

/**
 * Exposes `frame(seconds)` as the composition's seek hook once `setup` has resolved; frames
 * asked for before then wait for it.
 */
export const compose = (setup) => {
  const ready = setup();
  window.fosscut = {
    seek: async (seconds) => {
      const draw = await ready;
      await draw(Math.max(0, Number(seconds) || 0));
    },
  };
  return ready;
};
