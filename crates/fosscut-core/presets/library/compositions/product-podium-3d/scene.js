// 3D product on a podium, made by FossCut for its library (ADR-0042): a product (a picture on a
// card, a glTF model, or a bottle made here) rises onto a lit podium and turns with it.

import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { RoundedBoxGeometry } from 'three/addons/geometries/RoundedBoxGeometry.js';
import {
  Stage, color, compose, ease, glow, loadParams, loadTexture, mix, progress, studio,
} from './kit/stage.js';

// fosscut:defaults
const DEFAULTS = {
  "product": null,
  "product_color": "#E8B4A0",
  "podium_color": "#ECEAF2",
  "accent_color": "#7A5CFF",
  "floor_color": "#16141D",
  "product_scale": 1,
  "turn": 28,
  "in_at": 0.2,
  "in_duration": 1.2,
  "push": 0.18,
  "transparent": false,
  "background": "#121019",
  "exposure": 1,
  "tone_mapping": "aces",
  "bloom": { "strength": 0.9, "radius": 0.6, "threshold": 0.9 },
  "depth_of_field": { "aperture": 0.35, "max_blur": 0.01 },
  "motion_blur": { "samples": 8, "shutter": 0.5 }
};
// fosscut:end

const PRODUCT_HEIGHT = 1.5;

/** A profile for a lathe: a cylinder of `radius` and `height` with rounded edges. */
const drum = (radius, height, round) => {
  const points = [new THREE.Vector2(0, 0)];
  const arc = (cx, cy, from, to) => {
    for (let i = 0; i <= 8; i += 1) {
      const a = mix(from, to, i / 8);
      points.push(new THREE.Vector2(cx + Math.cos(a) * round, cy + Math.sin(a) * round));
    }
  };
  arc(radius - round, round, -Math.PI / 2, 0);
  arc(radius - round, height - round, 0, Math.PI / 2);
  points.push(new THREE.Vector2(0, height));
  return new THREE.LatheGeometry(points, 96);
};

/** FossCut's own product: a glossy bottle with a metal cap, standing on y = 0. */
const bottle = (P) => {
  const group = new THREE.Group();
  const profile = [
    [0, 0], [0.28, 0], [0.31, 0.02], [0.32, 0.06], [0.32, 0.78], [0.3, 0.86], [0.24, 0.94],
    [0.15, 0.99], [0.115, 1.03], [0.115, 1.08],
  ].map(([x, y]) => new THREE.Vector2(x, y));
  const body = new THREE.Mesh(
    new THREE.LatheGeometry(profile, 96),
    new THREE.MeshPhysicalMaterial({
      color: color(P.product_color), roughness: 0.18, metalness: 0, clearcoat: 1, clearcoatRoughness: 0.06, sheen: 0.4, sheenColor: color('#ffffff'),
    }),
  );
  body.castShadow = true;
  const cap = new THREE.Mesh(
    drum(0.135, 0.3, 0.025),
    new THREE.MeshPhysicalMaterial({ color: color('#D9C29A'), roughness: 0.22, metalness: 1 }),
  );
  cap.position.y = 1.06;
  cap.castShadow = true;
  const label = new THREE.Mesh(
    new THREE.CylinderGeometry(0.3225, 0.3225, 0.34, 96, 1, true, -0.9, 1.8),
    new THREE.MeshPhysicalMaterial({ color: color('#FFFFFF'), roughness: 0.5, metalness: 0, transparent: true, opacity: 0.92 }),
  );
  label.position.y = 0.42;
  group.add(body, cap, label);
  group.scale.setScalar(PRODUCT_HEIGHT / 1.36);
  return group;
};

/** A picture as a standing card. */
const card = async (slot) => {
  const texture = await loadTexture(slot.src);
  if (!texture) {
    return null;
  }
  const ratio = (texture.image.width || 1) / (texture.image.height || 1);
  const height = PRODUCT_HEIGHT;
  const width = Math.min(height * ratio, 1.7);
  const fitted = width / ratio;
  const group = new THREE.Group();
  const edge = new THREE.Mesh(
    new RoundedBoxGeometry(width + 0.06, fitted + 0.06, 0.04, 4, 0.02),
    new THREE.MeshPhysicalMaterial({ color: 0xffffff, roughness: 0.35, clearcoat: 0.6 }),
  );
  edge.castShadow = true;
  const face = new THREE.Mesh(
    new THREE.PlaneGeometry(width, fitted),
    new THREE.MeshStandardMaterial({ map: texture, roughness: 0.45, metalness: 0 }),
  );
  face.position.z = 0.0205;
  const backFace = face.clone();
  backFace.rotation.y = Math.PI;
  backFace.position.z = -0.0205;
  group.add(edge, face, backFace);
  group.children.forEach((child) => { child.position.y += fitted / 2 + 0.03; });
  return group;
};

/** A glTF model, scaled to stand PRODUCT_HEIGHT tall on y = 0. */
const model = (slot) => new Promise((resolve) => {
  new GLTFLoader().load(slot.src, (gltf) => {
    const root = gltf.scene;
    const box = new THREE.Box3().setFromObject(root);
    const size = box.getSize(new THREE.Vector3());
    const scale = PRODUCT_HEIGHT / Math.max(size.y, size.x * 0.8, size.z * 0.8, 1e-6);
    root.scale.setScalar(scale);
    const centre = box.getCenter(new THREE.Vector3()).multiplyScalar(scale);
    root.position.set(-centre.x, -box.min.y * scale, -centre.z);
    root.traverse((child) => {
      if (child.isMesh) {
        child.castShadow = true;
      }
    });
    const group = new THREE.Group();
    group.add(root);
    resolve(group);
  }, undefined, () => resolve(null));
});

compose(async () => {
  const P = await loadParams(DEFAULTS);
  const stage = new Stage(P, { fov: 30, near: 0.1, far: 80 });
  const { scene, camera, renderer } = stage;
  scene.environment = studio(renderer, { key: 1.1, tint: '#fff6ee', rim: P.accent_color });
  scene.environmentIntensity = 0.8;

  // The podium: a wide drum, a turntable on it and a ring of light round its edge.
  const world = new THREE.Group();
  const podiumMaterial = new THREE.MeshPhysicalMaterial({
    color: color(P.podium_color), roughness: 0.3, metalness: 0, clearcoat: 0.7, clearcoatRoughness: 0.2,
  });
  const base = new THREE.Mesh(drum(1.05, 0.34, 0.04), podiumMaterial);
  base.castShadow = true;
  base.receiveShadow = true;
  const table = new THREE.Group();
  const top = new THREE.Mesh(drum(0.86, 0.12, 0.03), podiumMaterial);
  top.position.y = 0.34;
  top.receiveShadow = true;
  top.castShadow = true;
  table.add(top);
  const ring = new THREE.Mesh(
    new THREE.TorusGeometry(1.055, 0.011, 12, 160).rotateX(Math.PI / 2),
    new THREE.MeshBasicMaterial({ color: glow(P.accent_color, 6) }),
  );
  ring.position.y = 0.335;
  const foot = ring.clone();
  foot.scale.setScalar(0.995);
  foot.position.y = 0.012;
  world.add(base, table, ring, foot);

  const slot = P.product && typeof P.product === 'object' ? P.product : null;
  let product = null;
  if (slot && slot.kind === 'model') {
    product = await model(slot);
  } else if (slot && slot.kind === 'image') {
    product = await card(slot);
  }
  if (!product) {
    product = bottle(P);
  }
  const holder = new THREE.Group();
  holder.add(product);
  holder.position.y = 0.46;
  table.add(holder);
  scene.add(world);

  // The floor reflects the podium: a mirrored copy under a glossy, slightly clear floor.
  if (P.transparent) {
    const shadow = new THREE.Mesh(new THREE.PlaneGeometry(30, 30).rotateX(-Math.PI / 2), new THREE.ShadowMaterial({ opacity: 0.4 }));
    shadow.receiveShadow = true;
    scene.add(shadow);
  } else {
    const mirror = world.clone();
    mirror.scale.y = -1;
    scene.add(mirror);
    world.userData.mirror = mirror;
    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(30, 96).rotateX(-Math.PI / 2),
      new THREE.MeshPhysicalMaterial({
        color: color(P.floor_color), roughness: 0.22, metalness: 0, transparent: true, opacity: 0.9, clearcoat: 0.6,
      }),
    );
    floor.receiveShadow = true;
    scene.add(floor);
    scene.fog = new THREE.Fog(color(P.background), 6, 16);
  }

  // Light: a soft key from above front with shadows, and rims in the accent colour.
  const key = new THREE.SpotLight(0xffffff, 60, 24, THREE.MathUtils.degToRad(44), 1, 1.4);
  key.position.set(-2.2, 6, 3.4);
  key.target.position.set(0, 0.6, 0);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.radius = 5;
  key.shadow.bias = -0.0004;
  const rimLeft = new THREE.DirectionalLight(color(P.accent_color), 2.6);
  rimLeft.position.set(-4, 2.5, -4);
  const rimRight = new THREE.DirectionalLight(0xffffff, 1.2);
  rimRight.position.set(4, 2, -3);
  scene.add(key, key.target, rimLeft, rimRight);

  const tan = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const distance = Math.max(2.3 / 0.8 / 2 / (tan * camera.aspect), 2.3 / 0.62 / 2 / tan);
  const look = new THREE.Vector3(0, 0.9, 0);

  const pose = (t) => {
    const p = progress(t, P.in_at, P.in_duration);
    const grow = ease.outBack(p, 1.1);
    holder.visible = p > 0;
    holder.scale.setScalar(Math.max(1e-3, grow) * P.product_scale);
    holder.position.y = 0.46 + 0.5 * (1 - ease.outCubic(p));
    holder.rotation.y = -1.4 * (1 - ease.outCubic(p));
    table.rotation.y = THREE.MathUtils.degToRad(P.turn) * t;
    const mirror = world.userData.mirror;
    if (mirror) {
      // The copy follows the turntable and the product.
      mirror.children[1].rotation.y = table.rotation.y;
      const copy = mirror.children[1].children[1];
      copy.visible = holder.visible;
      copy.scale.copy(holder.scale);
      copy.position.y = holder.position.y;
      copy.rotation.y = holder.rotation.y;
    }
    const push = ease.inOutSine(progress(t, 0, Math.max(P.in_at + P.in_duration + 3, 1)));
    const d = distance * (1 - P.push * push);
    camera.position.set(Math.sin(0.25 - 0.2 * push) * d, look.y + d * 0.36, Math.cos(0.25 - 0.2 * push) * d);
    camera.lookAt(look);
    camera.updateMatrixWorld();
    stage.focus.set(0, 0.46 + PRODUCT_HEIGHT * 0.5 * P.product_scale, 0);
  };

  return (t) => stage.render(t, pose);
});
