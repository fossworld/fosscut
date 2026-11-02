// 3D route map, made by FossCut for its library (ADR-0042): a city grid of dark blocks from a
// seed, a route drawn on along its streets with a dot at its head, and a pin at its end.

import * as THREE from 'three';
import {
  Stage, clamp, color, compose, ease, glow, loadParams, mix, progress, random, softDisc, studio,
} from './kit/stage.js';

// fosscut:defaults
const DEFAULTS = {
  "seed": 7,
  "blocks": 20,
  "route": [[-4, -6], [-4, -2], [1, -2], [1, 2], [4, 2], [4, 6]],
  "ground_color": "#0A0E16",
  "block_color": "#283550",
  "street_color": "#34507A",
  "window_color": "#FFC27A",
  "route_color": "#FF7A2F",
  "dot_color": "#FFF3E6",
  "pin_color": "#FF7A2F",
  "max_height": 2.4,
  "draw_start": 0.6,
  "draw_duration": 4.2,
  "pin_delay": 0.15,
  "tilt_from": 72,
  "tilt_to": 52,
  "push": 0.26,
  "turn": 20,
  "transparent": false,
  "background": "#06080D",
  "exposure": 1,
  "tone_mapping": "aces",
  "bloom": { "strength": 1.2, "radius": 0.6, "threshold": 0.9 },
  "depth_of_field": { "aperture": 0.5, "max_blur": 0.012 },
  "motion_blur": { "samples": 8, "shutter": 0.5 }
};
// fosscut:end

const PITCH = 1.32;
const BLOCK = 1.0;

/** How far the point x, z is from the polyline through `corners`. */
const distanceToRoute = (x, z, corners) => {
  let best = Infinity;
  for (let i = 1; i < corners.length; i += 1) {
    const a = corners[i - 1];
    const b = corners[i];
    const dx = b.x - a.x;
    const dz = b.z - a.z;
    const length = dx * dx + dz * dz;
    const s = length > 0 ? clamp(((x - a.x) * dx + (z - a.z) * dz) / length) : 0;
    best = Math.min(best, Math.hypot(x - (a.x + dx * s), z - (a.z + dz * s)));
  }
  return best;
};

/** Boxes of the blocks: [x, z, width, depth, height] from the seed, low along the route. */
const buildings = (P, corners) => {
  const rand = random(P.seed);
  const n = Math.max(2, Math.round(P.blocks));
  const half = Math.floor(n / 2);
  const reach = (n / 2) * PITCH;
  const boxes = [];
  const parks = [];
  for (let i = -half; i < n - half; i += 1) {
    for (let j = -half; j < n - half; j += 1) {
      const cx = (i + 0.5) * PITCH;
      const cz = (j + 0.5) * PITCH;
      const centre = 1 - clamp(Math.hypot(cx, cz) / reach);
      if (rand() < 0.1) {
        parks.push([cx, cz]);
        continue;
      }
      const split = rand() < 0.55 ? 2 : 1;
      const lot = BLOCK / split;
      for (let a = 0; a < split; a += 1) {
        for (let b = 0; b < split; b += 1) {
          const inset = 0.04 + rand() * 0.05;
          const x = cx - BLOCK / 2 + lot * (a + 0.5);
          const z = cz - BLOCK / 2 + lot * (b + 0.5);
          const near = clamp((distanceToRoute(x, z, corners) - 0.4) / (1.6 * PITCH));
          const height = 0.1 + rand() ** 2 * P.max_height * (0.35 + 0.65 * centre) * mix(0.18, 1, near);
          boxes.push([
            x,
            z,
            lot - inset * 2,
            lot - inset * 2,
            height,
          ]);
        }
      }
    }
  }
  return { boxes, parks, n, half };
};

/** A box of unit size standing on the ground, darker towards its foot. */
const blockGeometry = () => {
  const geometry = new THREE.BoxGeometry(1, 1, 1);
  geometry.translate(0, 0.5, 0);
  const position = geometry.getAttribute('position');
  const normal = geometry.getAttribute('normal');
  const shade = [];
  for (let i = 0; i < position.count; i += 1) {
    const top = normal.getY(i) > 0.5;
    const v = top ? 1.0 : mix(0.32, 0.78, position.getY(i));
    shade.push(v, v, v);
  }
  geometry.setAttribute('color', new THREE.Float32BufferAttribute(shade, 3));
  return geometry;
};

/** The route along the streets through `points` (street crossings), with rounded corners,
 * and the corners it turns at. */
const routeCurve = (points) => {
  const corners = [];
  const world = (p) => new THREE.Vector3(p[0] * PITCH, 0.06, p[1] * PITCH);
  for (let i = 0; i < points.length; i += 1) {
    const here = world(points[i]);
    if (i > 0) {
      const last = corners[corners.length - 1];
      if (Math.abs(last.x - here.x) > 1e-6 && Math.abs(last.z - here.z) > 1e-6) {
        corners.push(new THREE.Vector3(here.x, here.y, last.z));
      }
    }
    if (!corners.length || corners[corners.length - 1].distanceTo(here) > 1e-6) {
      corners.push(here);
    }
  }
  const path = new THREE.CurvePath();
  if (corners.length < 2) {
    corners.push(corners[0].clone().add(new THREE.Vector3(PITCH, 0, 0)));
  }
  let from = corners[0].clone();
  for (let i = 1; i < corners.length - 1; i += 1) {
    const corner = corners[i];
    const before = corners[i - 1];
    const after = corners[i + 1];
    const r = Math.min(0.42 * PITCH, corner.distanceTo(before) / 2, corner.distanceTo(after) / 2);
    const enter = corner.clone().add(before.clone().sub(corner).setLength(r));
    const leave = corner.clone().add(after.clone().sub(corner).setLength(r));
    if (from.distanceTo(enter) > 1e-6) {
      path.add(new THREE.LineCurve3(from, enter));
    }
    path.add(new THREE.QuadraticBezierCurve3(enter, corner.clone(), leave));
    from = leave;
  }
  path.add(new THREE.LineCurve3(from, corners[corners.length - 1].clone()));
  return { path, corners };
};

/** Lights some windows on the sides of the blocks, by a hash of where they are. */
const windows = (material, hex) => {
  const lit = color(hex).multiplyScalar(2.2);
  material.onBeforeCompile = (shader) => {
    shader.uniforms.windowColor = { value: lit };
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vCity;\nvarying vec3 vSide;')
      .replace('#include <begin_vertex>', `#include <begin_vertex>
        vec4 city = vec4(transformed, 1.0);
        #ifdef USE_INSTANCING
          city = instanceMatrix * city;
        #endif
        vCity = (modelMatrix * city).xyz;
        vSide = normal;`);
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', '#include <common>\nuniform vec3 windowColor;\nvarying vec3 vCity;\nvarying vec3 vSide;')
      .replace('#include <emissivemap_fragment>', `#include <emissivemap_fragment>
        if (abs(vSide.y) < 0.5 && vCity.y > 0.06) {
          float along = abs(vSide.x) > 0.5 ? vCity.z : vCity.x;
          vec2 cell = vec2(along / 0.085, vCity.y / 0.11);
          vec2 inside = fract(cell);
          float pane = step(0.22, inside.x) * step(inside.x, 0.78) * step(0.25, inside.y) * step(inside.y, 0.7);
          vec2 id = floor(cell) + floor(vCity.xz * 3.1);
          float on = step(0.83, fract(sin(dot(id, vec2(12.9898, 78.233))) * 43758.5453));
          // Windows smaller than a pixel fade to their average glow instead of flickering.
          float cellPixels = 1.0 / max(fwidth(cell.x), fwidth(cell.y));
          float sharp = smoothstep(2.0, 5.0, cellPixels);
          totalEmissiveRadiance += windowColor * mix(0.17 * 0.25, pane * on, sharp);
        }`);
  };
  return material;
};

/** A map pin standing on its point at the origin, `height` tall. */
const pinMesh = (hex, height) => {
  const head = height * 0.36;
  const profile = [];
  const steps = 48;
  // A drop: a circle of radius `head` around the top, closing to a point at the bottom.
  const centre = height - head;
  const tangent = Math.asin(head / centre);
  for (let i = 0; i <= steps; i += 1) {
    const a = mix(-Math.PI / 2 + tangent, Math.PI / 2, i / steps);
    profile.push(new THREE.Vector2(Math.cos(a) * head, centre + Math.sin(a) * head));
  }
  profile.unshift(new THREE.Vector2(0, 0));
  const body = new THREE.Mesh(
    new THREE.LatheGeometry(profile, 48),
    new THREE.MeshPhysicalMaterial({
      color: color(hex),
      emissive: color(hex),
      emissiveIntensity: 0.55,
      roughness: 0.28,
      metalness: 0.05,
      clearcoat: 1,
      clearcoatRoughness: 0.15,
    }),
  );
  body.castShadow = true;
  const dot = new THREE.Mesh(
    new THREE.SphereGeometry(head * 0.42, 32, 16),
    new THREE.MeshStandardMaterial({ color: 0xffffff, emissive: 0xffffff, emissiveIntensity: 1.6 }),
  );
  dot.position.set(0, centre, head * 0.72);
  dot.scale.set(1, 1, 0.45);
  const pin = new THREE.Group();
  pin.add(body, dot);
  return pin;
};

compose(async () => {
  const P = await loadParams(DEFAULTS);
  const stage = new Stage(P, { fov: 32, near: 0.5, far: 120 });
  const { scene, camera, renderer } = stage;
  scene.environment = studio(renderer, { key: 0.6, tint: '#c9d8ff', rim: '#ffb27a' });
  scene.environmentIntensity = 0.7;
  if (!P.transparent) {
    scene.fog = new THREE.FogExp2(color(P.background), 0.035);
  }

  const points = Array.isArray(P.route) && P.route.length ? P.route : DEFAULTS.route;
  const { path: curve, corners } = routeCurve(points);
  const city = buildings(P, corners);
  const extent = (city.n / 2 + 1.5) * PITCH;

  // Ground and streets.
  // When transparent, only the shadows on the ground show.
  const ground = new THREE.Mesh(
    new THREE.PlaneGeometry(extent * 2.4, extent * 2.4),
    P.transparent
      ? new THREE.ShadowMaterial({ opacity: 0.5 })
      : new THREE.MeshStandardMaterial({ color: color(P.ground_color), roughness: 0.92, metalness: 0 }),
  );
  ground.rotation.x = -Math.PI / 2;
  ground.receiveShadow = true;
  scene.add(ground);
  const streetCount = (city.n + 1) * 2;
  const streets = new THREE.InstancedMesh(
    new THREE.PlaneGeometry(1, 1).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: color(P.street_color), fog: true }),
    streetCount,
  );
  const matrix = new THREE.Matrix4();
  const length = city.n * PITCH + PITCH;
  for (let k = 0; k <= city.n; k += 1) {
    const at = (k - city.half) * PITCH;
    matrix.compose(new THREE.Vector3(at, 0.005, 0), new THREE.Quaternion(), new THREE.Vector3(0.035, 1, length));
    streets.setMatrixAt(k * 2, matrix);
    matrix.compose(new THREE.Vector3(0, 0.005, at), new THREE.Quaternion(), new THREE.Vector3(length, 1, 0.035));
    streets.setMatrixAt(k * 2 + 1, matrix);
  }
  scene.add(streets);

  // Blocks.
  const blocks = new THREE.InstancedMesh(
    blockGeometry(),
    windows(new THREE.MeshStandardMaterial({ color: 0xffffff, vertexColors: true, roughness: 0.62, metalness: 0.2 }), P.window_color),
    city.boxes.length,
  );
  const base = color(P.block_color);
  const shadeRand = random(P.seed + 101);
  city.boxes.forEach(([x, z, w, d, h], i) => {
    matrix.compose(new THREE.Vector3(x, 0, z), new THREE.Quaternion(), new THREE.Vector3(w, h, d));
    blocks.setMatrixAt(i, matrix);
    blocks.setColorAt(i, base.clone().multiplyScalar(0.8 + shadeRand() * 0.45));
  });
  blocks.castShadow = true;
  blocks.receiveShadow = true;
  scene.add(blocks);
  const parks = new THREE.InstancedMesh(
    new THREE.BoxGeometry(1, 0.05, 1).translate(0, 0.025, 0),
    new THREE.MeshStandardMaterial({ color: color(P.block_color).lerp(new THREE.Color(0x0f2a22), 0.6), roughness: 0.95 }),
    Math.max(1, city.parks.length),
  );
  parks.count = city.parks.length;
  city.parks.forEach(([x, z], i) => {
    matrix.compose(new THREE.Vector3(x, 0, z), new THREE.Quaternion(), new THREE.Vector3(BLOCK * 0.92, 1, BLOCK * 0.92));
    parks.setMatrixAt(i, matrix);
  });
  parks.receiveShadow = true;
  scene.add(parks);

  // Light: a cool moon with shadows, a faint sky, and the route's own warm light.
  const moon = new THREE.DirectionalLight(0xc4d2ff, 1.6);
  moon.position.set(-8, 14, 6);
  moon.castShadow = true;
  moon.shadow.mapSize.set(2048, 2048);
  const cover = extent;
  Object.assign(moon.shadow.camera, { left: -cover, right: cover, top: cover, bottom: -cover, near: 1, far: 40 });
  moon.shadow.bias = -0.0005;
  moon.shadow.normalBias = 0.02;
  scene.add(moon, moon.target);
  scene.add(new THREE.HemisphereLight(0x5a6c94, 0x07080c, 0.9));

  // The route.
  const segments = Math.max(64, Math.round(curve.getLength() * 60));
  const radial = 10;
  const tube = new THREE.Mesh(
    new THREE.TubeGeometry(curve, segments, 0.055, radial, false),
    new THREE.MeshBasicMaterial({ color: glow(P.route_color, 5), fog: false, toneMapped: false }),
  );
  const halo = new THREE.Mesh(
    new THREE.TubeGeometry(curve, segments, 0.16, radial, false),
    new THREE.MeshBasicMaterial({
      color: glow(P.route_color, 1.2), transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false, fog: false,
    }),
  );
  scene.add(tube, halo);
  const lit = new THREE.PointLight(color(P.route_color), 6, 5, 1.6);
  scene.add(lit);
  const head = new THREE.Mesh(
    new THREE.SphereGeometry(0.12, 32, 16),
    new THREE.MeshBasicMaterial({ color: glow(P.dot_color, 6), fog: false }),
  );
  const flare = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1),
    new THREE.MeshBasicMaterial({
      map: softDisc(128, 0.6), color: glow(P.route_color, 2.5), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, fog: false,
    }),
  );
  const ring = new THREE.Mesh(
    new THREE.RingGeometry(0.8, 1, 64).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: glow(P.route_color, 2), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false, fog: false }),
  );
  scene.add(head, flare, ring);
  const start = new THREE.Mesh(
    new THREE.RingGeometry(0.13, 0.2, 48).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: glow(P.route_color, 3), fog: false }),
  );
  start.position.copy(curve.getPointAt(0)).setY(0.03);
  scene.add(start);

  // The pin, its shadow and the ring it sends out when it lands.
  const end = curve.getPointAt(1);
  const pin = pinMesh(P.pin_color, 0.95);
  scene.add(pin);
  const pinShadow = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ map: softDisc(128, 1), color: 0x000000, transparent: true, depthWrite: false }),
  );
  pinShadow.position.set(end.x, 0.02, end.z);
  const landing = ring.clone();
  landing.material = ring.material.clone();
  scene.add(pinShadow, landing);

  const drawEnd = P.draw_start + P.draw_duration;
  const pinAt = drawEnd + P.pin_delay;
  const centre = new THREE.Box3().setFromPoints(curve.getSpacedPoints(64)).getCenter(new THREE.Vector3());
  const span = new THREE.Box3().setFromPoints(curve.getSpacedPoints(64)).getSize(new THREE.Vector3()).length();
  const duration = Math.max(drawEnd + 1.5, 1);
  // The camera looks along the route from behind its start, a little to the side.
  const first = curve.getPointAt(0);
  const behind = Math.atan2(first.x - end.x, first.z - end.z) + 0.2;
  const headAt = (t) => ease.inOutCubic(progress(t, P.draw_start, P.draw_duration));
  const look = new THREE.Vector3();

  const pose = (t) => {
    const drawn = headAt(t);
    const index = Math.round(drawn * segments) * radial * 6;
    tube.geometry.setDrawRange(0, index);
    halo.geometry.setDrawRange(0, index);
    const visible = drawn > 0;
    const at = curve.getPointAt(clamp(drawn, 0, 1));
    head.visible = visible && drawn < 1;
    flare.visible = head.visible;
    head.position.copy(at).setY(0.1);
    lit.position.copy(at).setY(0.6);
    lit.intensity = visible ? 6 : 0;
    const pulse = 1 + 0.15 * Math.sin(t * 9);
    flare.position.copy(head.position);
    flare.scale.setScalar(0.9 * pulse);
    ring.visible = head.visible;
    const cycle = ((t - P.draw_start) * 1.4) % 1;
    ring.position.copy(at).setY(0.03);
    ring.scale.setScalar(0.15 + 0.5 * cycle);
    ring.material.opacity = (1 - cycle) * 0.8;

    // The pin drops in on a spring after the route arrives.
    const since = t - pinAt;
    const fall = 0.32;
    const height = since < fall ? 2.6 * (1 - (since / fall) ** 2) : 0.3 * Math.exp(-(since - fall) * 7) * Math.abs(Math.sin((since - fall) * 13));
    const drop = clamp(since / fall);
    pin.visible = since > 0;
    pin.position.set(end.x, Math.max(0, height), end.z);
    const after = since - fall;
    const squash = after > 0 ? 1 - 0.22 * Math.exp(-after * 8) * Math.cos(after * 22) : 1 + 0.08 * drop;
    pin.scale.set(1 / Math.sqrt(squash), squash, 1 / Math.sqrt(squash));
    pin.rotation.y = mix(-1.2, 0, ease.outCubic(clamp(since / 0.8)));
    pinShadow.visible = pin.visible;
    pinShadow.scale.setScalar(mix(1.4, 0.75, drop));
    pinShadow.material.opacity = 0.7 * drop;
    const landed = since - fall;
    landing.visible = landed > 0 && landed < 1.2;
    landing.position.set(end.x, 0.03, end.z);
    landing.scale.setScalar(0.3 + 1.6 * ease.outCubic(clamp(landed / 1.2)));
    landing.material.opacity = 0.9 * (1 - clamp(landed / 1.2));

    // The camera tilts down and pushes in, turning slowly, and leans after the head.
    const whole = ease.inOutSine(progress(t, 0, duration));
    const tilt = THREE.MathUtils.degToRad(mix(P.tilt_from, P.tilt_to, whole));
    const turn = behind + THREE.MathUtils.degToRad(-P.turn / 2 + P.turn * whole);
    const distance = (span * 0.8 + 5) * (1 - P.push * whole);
    // The camera drifts from the route's start to its end over the whole shot and leans
    // after the head, so it keeps the head in view without racing it.
    const lag = curve.getPointAt(clamp(headAt(t - 0.5), 0, 1));
    look.copy(first).lerp(end, whole).lerp(lag, 0.45);
    camera.position.set(
      look.x + Math.cos(tilt) * Math.sin(turn) * distance,
      look.y + Math.sin(tilt) * distance,
      look.z + Math.cos(tilt) * Math.cos(turn) * distance,
    );
    camera.lookAt(look);
    camera.updateMatrixWorld();
    flare.quaternion.copy(camera.quaternion);
    moon.target.position.copy(look);
    moon.position.copy(look).add(new THREE.Vector3(-8, 14, 6));
    stage.focus.copy(head.visible ? head.position : pin.visible ? pin.position : look);
  };

  return (t) => stage.render(t, pose);
});
