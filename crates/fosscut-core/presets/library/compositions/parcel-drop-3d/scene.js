// 3D parcel drop, made by FossCut for its library (ADR-0042): a cardboard box with a logo
// on its front falls onto a floor, squashes, bounces and settles, throwing sparks.

import * as THREE from 'three';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import {
  Stage, clamp, color, compose, ease, glow, loadParams, mix, paintTexture, progress, random, softDisc, studio,
} from './kit/stage.js';

// fosscut:defaults
const DEFAULTS = {
  "logo": null,
  "box_color": "#F07A2B",
  "tape_color": "#F2D3A0",
  "floor_color": "#2A2F3A",
  "spark_color": "#FFB347",
  "box_width": 1.6,
  "box_height": 1.15,
  "box_depth": 1.25,
  "drop_at": 0.35,
  "fall": 0.5,
  "drop_height": 5,
  "spin": 14,
  "sparks": 64,
  "shake": 0.35,
  "transparent": false,
  "background": "#1B1F27",
  "exposure": 1,
  "tone_mapping": "aces",
  "bloom": { "strength": 1, "radius": 0.55, "threshold": 0.9 },
  "depth_of_field": { "aperture": 0.35, "max_blur": 0.012 },
  "motion_blur": { "samples": 16, "shutter": 0.5 }
};
// fosscut:end

// FossCut's own mark printed when no logo is given: a box with speed lines.
const MARK = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="720" viewBox="0 0 200 120">
<g fill="#1A1D24"><rect x="72" y="18" width="112" height="84" rx="14"/>
<rect x="18" y="34" width="42" height="11" rx="5.5"/><rect x="6" y="55" width="54" height="11" rx="5.5"/>
<rect x="26" y="76" width="34" height="11" rx="5.5"/></g>
<rect x="72" y="50" width="112" height="9" fill="#FFFFFF" opacity="0.9"/></svg>`;

/** A picture for the box's front: the logo slot's file, or the mark, sharp at any size. */
const loadPicture = (slot) => new Promise((resolve) => {
  const image = new Image();
  const src = slot && slot.src ? slot.src : `data:image/svg+xml;charset=utf-8,${encodeURIComponent(MARK)}`;
  const vector = !slot || slot.kind === 'svg' || /\.svg$/i.test(src);
  image.onload = () => {
    const ratio = (image.naturalWidth || 1) / (image.naturalHeight || 1);
    let texture;
    if (vector) {
      // Vector drawings are drawn large first, so they stay sharp up close.
      const canvas = document.createElement('canvas');
      canvas.width = 2048;
      canvas.height = Math.max(1, Math.round(2048 / ratio));
      canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
      texture = new THREE.CanvasTexture(canvas);
    } else {
      texture = new THREE.Texture(image);
    }
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.anisotropy = 8;
    texture.needsUpdate = true;
    resolve({ texture, ratio });
  };
  image.onerror = () => resolve(null);
  image.src = src;
});

/** Cardboard: fine fibres running across, and a little mottling, from the seed. */
const cardboard = () => {
  const rand = random(11);
  const lines = Array.from({ length: 256 }, () => rand());
  const blots = Array.from({ length: 24 }, () => [rand(), rand(), 0.04 + rand() * 0.1]);
  return paintTexture(256, (u, v) => {
    const fibre = lines[Math.floor(v * 255)] * 0.06 + lines[Math.floor((v * 3.7) % 1 * 255)] * 0.04;
    let blot = 0;
    for (const [x, y, r] of blots) {
      blot += Math.max(0, 1 - Math.hypot(u - x, v - y) / r) * 0.05;
    }
    const value = 0.9 + fibre - blot;
    return [value, value, value, 1];
  }, true);
};

/** Height of the box's bottom above the floor `s` seconds after it first lands. */
const bounce = (s, height) => {
  const arcs = [[0, 0.26, 0.11], [0.26, 0.15, 0.03], [0.41, 0.08, 0.008]];
  for (const [start, length, peak] of arcs) {
    if (s >= start && s < start + length) {
      const x = (s - start) / length;
      return 4 * peak * height * x * (1 - x);
    }
  }
  return 0;
};

compose(async () => {
  const P = await loadParams(DEFAULTS);
  const stage = new Stage(P, { fov: 30, near: 0.1, far: 80 });
  const { scene, camera, renderer } = stage;
  scene.environment = studio(renderer, { key: 1, tint: '#fff1e2', rim: '#cfe0ff' });
  scene.environmentIntensity = 0.55;

  const W = P.box_width;
  const H = P.box_height;
  const D = P.box_depth;
  const size = Math.max(W, H, D);

  // The floor; when transparent only its shadows show.
  const floor = new THREE.Mesh(
    new THREE.PlaneGeometry(60, 60).rotateX(-Math.PI / 2),
    P.transparent
      ? new THREE.ShadowMaterial({ opacity: 0.45 })
      : new THREE.MeshStandardMaterial({ color: color(P.floor_color), roughness: 0.55, metalness: 0 }),
  );
  floor.receiveShadow = true;
  scene.add(floor);
  if (!P.transparent) {
    scene.fog = new THREE.Fog(color(P.background), size * 5, size * 15);
  }

  // The box: rounded cardboard with tape over the top and the logo on its front.
  const box = new THREE.Group();
  const body = new THREE.Mesh(
    new RoundedBoxGeometry(W, H, D, 5, Math.min(W, H, D) * 0.05),
    new THREE.MeshPhysicalMaterial({
      color: color(P.box_color),
      map: cardboard(),
      roughness: 0.82,
      metalness: 0,
      sheen: 0.3,
      sheenRoughness: 0.8,
      sheenColor: color(P.box_color),
    }),
  );
  body.castShadow = true;
  body.receiveShadow = true;
  box.add(body);
  const tapeMaterial = new THREE.MeshPhysicalMaterial({
    color: color(P.tape_color), roughness: 0.32, metalness: 0, clearcoat: 0.6, clearcoatRoughness: 0.3,
  });
  const tapeWidth = W * 0.22;
  const top = new THREE.Mesh(new THREE.BoxGeometry(tapeWidth, 0.006, D * 1.004), tapeMaterial);
  top.position.y = H / 2 + 0.002;
  const lip = H * 0.22;
  const front = new THREE.Mesh(new THREE.BoxGeometry(tapeWidth, lip, 0.006), tapeMaterial);
  front.position.set(0, H / 2 - lip / 2, D / 2 + 0.002);
  const back = front.clone();
  back.position.z = -front.position.z;
  box.add(top, front, back);
  const picture = await loadPicture(P.logo);
  if (picture) {
    const room = [W * 0.62, H * 0.46];
    let pw = room[0];
    let ph = pw / picture.ratio;
    if (ph > room[1]) {
      ph = room[1];
      pw = ph * picture.ratio;
    }
    const logo = new THREE.Mesh(
      new THREE.PlaneGeometry(pw, ph),
      new THREE.MeshStandardMaterial({
        map: picture.texture, transparent: true, roughness: 0.6, metalness: 0, polygonOffset: true, polygonOffsetFactor: -2,
      }),
    );
    logo.position.set(0, -H * 0.08, D / 2 + 0.004);
    box.add(logo);
  }
  scene.add(box);

  // The contact shadow under the box, darker as it nears the floor.
  const contact = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ map: softDisc(128, 0.8), color: 0x000000, transparent: true, depthWrite: false }),
  );
  contact.position.y = 0.004;
  scene.add(contact);

  // Dust and a ring of light where it lands.
  const dust = new THREE.Mesh(
    new THREE.RingGeometry(0.7, 1, 96).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: color(P.floor_color).lerp(new THREE.Color(1, 1, 1), 0.35), transparent: true, depthWrite: false }),
  );
  dust.position.y = 0.006;
  const wave = new THREE.Mesh(
    new THREE.RingGeometry(0.93, 1, 128).rotateX(-Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: glow(P.spark_color, 3), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false }),
  );
  wave.position.y = 0.008;
  scene.add(dust, wave);

  // Sparks: streaks thrown from the box's foot when it lands, from the seed.
  const count = Math.max(0, Math.round(P.sparks));
  const rand = random(29);
  const sparks = Array.from({ length: count }, () => {
    const angle = rand() * Math.PI * 2;
    const rise = mix(0.12, 0.85, rand());
    const speed = mix(2.2, 5.5, rand()) * size * 0.75;
    const corner = new THREE.Vector3(Math.cos(angle) * W * 0.5, 0.02, Math.sin(angle) * D * 0.5);
    return {
      from: corner,
      velocity: new THREE.Vector3(Math.cos(angle) * Math.cos(rise), Math.sin(rise), Math.sin(angle) * Math.cos(rise)).multiplyScalar(speed),
      life: mix(0.35, 0.95, rand()),
      thickness: mix(0.6, 1.3, rand()),
      delay: rand() * 0.05,
    };
  });
  const streaks = new THREE.InstancedMesh(
    new THREE.CapsuleGeometry(0.5, 1, 2, 6).rotateZ(Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: glow(P.spark_color, 7), fog: false }),
    Math.max(1, count),
  );
  streaks.count = count;
  streaks.frustumCulled = false;
  scene.add(streaks);

  // Light: a warm key with soft shadows and a cool rim.
  const key = new THREE.DirectionalLight(0xfff1e0, 2.4);
  key.position.set(-3 * size, 6 * size, 4 * size);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.radius = 6;
  Object.assign(key.shadow.camera, { left: -size * 3, right: size * 3, top: size * 3, bottom: -size * 3, near: 0.5, far: size * 20 });
  key.shadow.bias = -0.0004;
  key.shadow.normalBias = 0.02;
  const rim = new THREE.DirectionalLight(0xbcd2ff, 1.4);
  rim.position.set(3 * size, 2 * size, -5 * size);
  scene.add(key, rim, new THREE.HemisphereLight(0xdfe8ff, 0x24201c, 0.35));

  const landAt = P.drop_at + P.fall;
  const startHeight = P.drop_height * H;
  const duration = landAt + 3;
  // The camera frames the box by the frame's narrower side.
  const halfWide = Math.atan(Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * camera.aspect);
  const halfTall = THREE.MathUtils.degToRad(camera.fov / 2);
  const reach = Math.max(W * 1.15, D) / (0.78 * Math.tan(Math.min(halfWide, halfTall))) / 2 + D / 2;
  const look = new THREE.Vector3();
  const matrix = new THREE.Matrix4();
  const quaternion = new THREE.Quaternion();
  const direction = new THREE.Vector3();
  const xAxis = new THREE.Vector3(1, 0, 0);

  const pose = (t) => {
    const s = t - landAt;
    let lift;
    let squash = 0;
    if (s < 0) {
      const f = clamp((t - P.drop_at) / P.fall);
      lift = t < P.drop_at ? startHeight * 1.4 : startHeight * (1 - f * f);
      squash = -0.06 * f;
    } else {
      lift = bounce(s, H);
      squash = 0.2 * Math.exp(-s * 9) * Math.cos(s * 26);
    }
    const sy = 1 - squash;
    const sxz = 1 / Math.sqrt(sy);
    box.scale.set(sxz, sy, sxz);
    box.position.set(0, lift + (H / 2) * sy, 0);
    const spinLeft = s < 0 ? 1 - ease.outCubic(clamp((t - P.drop_at) / P.fall)) : 0;
    box.rotation.set(
      THREE.MathUtils.degToRad(6) * spinLeft,
      THREE.MathUtils.degToRad(P.spin) * spinLeft,
      s > 0 ? 0.035 * Math.exp(-s * 5) * Math.sin(s * 17) : THREE.MathUtils.degToRad(-4) * spinLeft,
    );

    const near = clamp(1 - lift / (startHeight * 0.9));
    contact.scale.set(W * mix(2.4, 1.45, near), 1, D * mix(2.4, 1.5, near));
    contact.material.opacity = 0.85 * near * near;

    const since = s;
    dust.visible = since > 0 && since < 1.4;
    wave.visible = since > 0 && since < 0.8;
    if (since > 0) {
      const spread = ease.outCubic(clamp(since / 1.4));
      dust.scale.setScalar(size * mix(0.7, 2.1, spread));
      dust.material.opacity = 0.35 * (1 - spread);
      const run = ease.outQuint(clamp(since / 0.8));
      wave.scale.setScalar(size * mix(0.6, 2.6, run));
      wave.material.opacity = 1 - run;
    }

    const gravity = -9.8 * size * 0.55;
    sparks.forEach((spark, i) => {
      const age = since - spark.delay;
      if (age <= 0 || age >= spark.life) {
        matrix.makeScale(0, 0, 0);
        streaks.setMatrixAt(i, matrix);
        return;
      }
      const at = spark.from.clone().addScaledVector(spark.velocity, age);
      at.y = Math.max(0.01, at.y + 0.5 * gravity * age * age);
      direction.copy(spark.velocity);
      direction.y += gravity * age;
      const speed = direction.length();
      quaternion.setFromUnitVectors(xAxis, direction.normalize());
      const fade = 1 - age / spark.life;
      const thick = 0.016 * spark.thickness * size * fade;
      matrix.compose(at, quaternion, new THREE.Vector3(0.02 * speed * fade + thick, thick, thick));
      streaks.setMatrixAt(i, matrix);
    });
    streaks.instanceMatrix.needsUpdate = true;

    // The camera pushes in slowly and shakes when the box lands.
    const push = ease.inOutSine(progress(t, 0, duration));
    const shake = since > 0 ? P.shake * 0.05 * size * Math.exp(-since * 6) : 0;
    look.set(
      shake * Math.sin(since * 47),
      H * 0.55 + shake * Math.sin(since * 39 + 1.3),
      0,
    );
    const distance = reach * mix(1.12, 0.94, push);
    camera.position.set(
      look.x + distance * Math.sin(0.42),
      look.y + distance * 0.32,
      distance * Math.cos(0.42),
    );
    camera.lookAt(look);
    camera.updateMatrixWorld();
    stage.focus.copy(box.position);
  };

  return (t) => stage.render(t, pose);
});
