// 3D phone with a live screen, made by FossCut for its library (ADR-0042): a generic phone
// rises, turns towards the camera and the camera pushes in, while its screen shows another
// web composition at its own clock time (or a picture or video), placed in 3D as HTML.

import * as THREE from 'three';
import { CSS3DObject, CSS3DRenderer } from 'three/addons/renderers/CSS3DRenderer.js';
import {
  Stage, clamp, color, compose, ease, glow, loadParams, mix, progress, softDisc, studio,
} from './kit/stage.js';

// fosscut:defaults
const DEFAULTS = {
  "screen": "shop-app",
  "screen_start": 0,
  "body_color": "#22252B",
  "frame_color": "#A7AFBA",
  "glow_color": "#5B8CFF",
  "glow": 1.2,
  "rise_at": 0,
  "rise_duration": 1.3,
  "turn_at": 0.7,
  "turn_duration": 1.8,
  "push_at": 2.2,
  "push_duration": 3.3,
  "start_angle": -32,
  "tilt": 16,
  "push": 0.15,
  "float": 0.3,
  "transparent": false,
  "background": "#0B0E14",
  "exposure": 1,
  "tone_mapping": "aces",
  "bloom": { "strength": 1, "radius": 0.65, "threshold": 0.9 },
  "depth_of_field": { "aperture": 0.25, "max_blur": 0.01 },
  "motion_blur": { "samples": 8, "shutter": 0.5 }
};
// fosscut:end

// The phone's size in scene units: 2 tall, with the proportions of a current phone.
const HEIGHT = 2;
const WIDTH = 0.976;
const DEPTH = 0.1;
const CORNER = 0.16;
const BEZEL = 0.032;

/** A rounded rectangle centred on the origin. */
const roundedRect = (w, h, r) => {
  const shape = new THREE.Shape();
  const x = -w / 2;
  const y = -h / 2;
  shape.moveTo(x + r, y);
  shape.lineTo(x + w - r, y);
  shape.quadraticCurveTo(x + w, y, x + w, y + r);
  shape.lineTo(x + w, y + h - r);
  shape.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
  shape.lineTo(x + r, y + h);
  shape.quadraticCurveTo(x, y + h, x, y + h - r);
  shape.lineTo(x, y + r);
  shape.quadraticCurveTo(x, y, x + r, y);
  return shape;
};

/** The phone: a metal frame round a glass back and a black glass front, with a camera. */
const phoneBody = (P) => {
  const group = new THREE.Group();
  const bevel = 0.022;
  const geometry = new THREE.ExtrudeGeometry(roundedRect(WIDTH - bevel * 2, HEIGHT - bevel * 2, CORNER - bevel), {
    depth: DEPTH - bevel * 2,
    bevelEnabled: true,
    bevelThickness: bevel,
    bevelSize: bevel,
    bevelSegments: 8,
    curveSegments: 24,
  });
  geometry.translate(0, 0, -(DEPTH - bevel * 2) / 2);
  const back = new THREE.MeshPhysicalMaterial({
    color: color(P.body_color), roughness: 0.38, metalness: 0.1, clearcoat: 1, clearcoatRoughness: 0.2,
  });
  const metal = new THREE.MeshPhysicalMaterial({
    color: color(P.frame_color), roughness: 0.22, metalness: 1, clearcoat: 0.4,
  });
  const shell = new THREE.Mesh(geometry, [back, metal]);
  shell.castShadow = true;
  group.add(shell);
  // The front glass, black where the screen is off.
  const glass = new THREE.Mesh(
    new THREE.ShapeGeometry(roundedRect(WIDTH - 0.012, HEIGHT - 0.012, CORNER - 0.006), 24),
    new THREE.MeshPhysicalMaterial({ color: 0x030405, roughness: 0.06, metalness: 0, clearcoat: 1, clearcoatRoughness: 0.03 }),
  );
  glass.position.z = DEPTH / 2 + 0.0015;
  group.add(glass);
  // The camera island on the back.
  const island = new THREE.Mesh(
    new THREE.ExtrudeGeometry(roundedRect(0.36, 0.36, 0.09), { depth: 0.012, bevelEnabled: true, bevelThickness: 0.006, bevelSize: 0.006, bevelSegments: 4, curveSegments: 16 }),
    back,
  );
  island.position.set(-WIDTH / 2 + 0.25, HEIGHT / 2 - 0.25, -DEPTH / 2);
  island.rotation.y = Math.PI;
  group.add(island);
  const lensMaterial = new THREE.MeshPhysicalMaterial({ color: 0x050608, roughness: 0.05, metalness: 0.4, clearcoat: 1 });
  const ringMaterial = metal;
  for (const [dx, dy] of [[-0.075, 0.075], [0.075, 0.075], [-0.075, -0.075]]) {
    const ring = new THREE.Mesh(new THREE.CylinderGeometry(0.058, 0.058, 0.03, 32).rotateX(Math.PI / 2), ringMaterial);
    ring.position.set(island.position.x - dx, island.position.y + dy, -DEPTH / 2 - 0.03);
    const lens = new THREE.Mesh(new THREE.CircleGeometry(0.045, 32), lensMaterial);
    lens.position.set(ring.position.x, ring.position.y, -DEPTH / 2 - 0.0455);
    lens.rotation.y = Math.PI;
    group.add(ring, lens);
  }
  // Buttons on the sides.
  const button = (y, length, side) => {
    const mesh = new THREE.Mesh(new THREE.CapsuleGeometry(0.012, length, 4, 12), metal);
    mesh.position.set(side * (WIDTH / 2 + 0.006), y, 0);
    group.add(mesh);
  };
  button(0.42, 0.22, 1);
  button(0.5, 0.12, -1);
  button(0.32, 0.12, -1);
  return group;
};

/**
 * The screen's HTML: a frame of another composition, a picture or a video. `start()` loads
 * it once the element is in the document.
 */
const screenElement = (slot, width, height) => {
  const holder = document.createElement('div');
  Object.assign(holder.style, {
    width: `${width}px`, height: `${height}px`, overflow: 'hidden', background: '#000', backfaceVisibility: 'hidden',
  });
  // Media sit in a shadow root, which FossCut's clock leaves alone, so the screen keeps its own time.
  const root = holder.attachShadow({ mode: 'open' });
  const fill = 'display:block;width:100%;height:100%;border:0;object-fit:cover;';
  const sheen = document.createElement('div');
  sheen.style.cssText = 'position:absolute;inset:0;pointer-events:none;mix-blend-mode:screen;';
  const wrap = document.createElement('div');
  wrap.style.cssText = 'position:relative;width:100%;height:100%;';
  root.appendChild(wrap);
  let show = async () => {};
  let start = async () => {};
  if (slot && slot.kind === 'composition') {
    const frame = document.createElement('iframe');
    frame.style.cssText = `${fill}width:${slot.width}px;height:${slot.height}px;transform-origin:0 0;transform:scale(${width / slot.width});`;
    frame.setAttribute('scrolling', 'no');
    const load = () => new Promise((resolve) => {
      frame.addEventListener('load', () => resolve(), { once: true });
      frame.src = slot.src;
    });
    wrap.appendChild(frame);
    start = async () => {
      await load();
      await frame.contentWindow.__fosscut.ready();
    };
    const last = Math.max(0, Math.floor((Number(slot.duration) || 1) * 1000) - 1);
    show = async (seconds) => {
      const k = Math.min(last, Math.max(0, Math.round(seconds * 1000)));
      if (await frame.contentWindow.__fosscut.show(k, 1000, 1) === 'reload') {
        await load();
        await frame.contentWindow.__fosscut.ready();
        await frame.contentWindow.__fosscut.show(k, 1000, 1);
      }
    };
  } else if (slot && slot.kind === 'video') {
    // The video stays out of the document and is drawn into a canvas once it has sought,
    // so the screen shows exactly the frame for its time.
    const video = document.createElement('video');
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    canvas.style.cssText = fill;
    wrap.appendChild(canvas);
    const context = canvas.getContext('2d');
    const paint = () => {
      const vw = video.videoWidth || width;
      const vh = video.videoHeight || height;
      const scale = Math.max(width / vw, height / vh);
      context.drawImage(video, (width - vw * scale) / 2, (height - vh * scale) / 2, vw * scale, vh * scale);
    };
    start = async () => {
      // Loaded whole as a blob, which the browser can seek anywhere in.
      const blob = await (await fetch(slot.src)).blob();
      await new Promise((resolve) => {
        video.addEventListener('loadeddata', resolve, { once: true });
        video.addEventListener('error', resolve, { once: true });
        video.src = URL.createObjectURL(blob);
      });
    };
    show = (seconds) => new Promise((resolve) => {
      const length = video.duration > 0 ? video.duration : 0;
      const target = length > 0 ? seconds % length : 0;
      const done = () => {
        paint();
        resolve();
      };
      if (Math.abs(video.currentTime - target) < 0.0005 && video.readyState >= 2) {
        done();
        return;
      }
      video.addEventListener('seeked', done, { once: true });
      video.addEventListener('error', () => resolve(), { once: true });
      video.currentTime = target;
    });
  } else if (slot && slot.kind === 'image') {
    const image = document.createElement('img');
    image.style.cssText = fill;
    wrap.appendChild(image);
    start = async () => {
      image.src = slot.src;
      await image.decode().catch(() => {});
    };
  }
  wrap.appendChild(sheen);
  return { holder, sheen, show, start };
};

compose(async () => {
  const P = await loadParams(DEFAULTS);
  const stage = new Stage(P, { fov: 30, near: 0.1, far: 60 });
  const { scene, camera, renderer } = stage;
  scene.environment = studio(renderer, { key: 1.1, tint: '#f4f7ff', rim: P.glow_color });
  scene.environmentIntensity = 0.9;

  const phone = new THREE.Group();
  phone.add(phoneBody(P));
  scene.add(phone);

  // The screen, as HTML in 3D on the front glass.
  const screenW = WIDTH - BEZEL * 2;
  const screenH = HEIGHT - BEZEL * 2;
  const slot = P.screen && typeof P.screen === 'object' ? P.screen : null;
  const pixels = slot && slot.kind === 'composition' && slot.width > 0 ? slot.width : 720;
  const pxW = pixels;
  const pxH = Math.round(pixels * (screenH / screenW));
  const screen = screenElement(slot, pxW, pxH);
  screen.holder.style.borderRadius = `${((CORNER - BEZEL) / screenW) * pxW}px`;
  const panel = new CSS3DObject(screen.holder);
  panel.scale.setScalar(screenW / pxW);
  panel.position.z = DEPTH / 2 + 0.002;
  phone.add(panel);
  const css = new CSS3DRenderer();
  css.setSize(stage.width, stage.height);
  Object.assign(css.domElement.style, { position: 'absolute', left: '0', top: '0' });
  document.body.appendChild(css.domElement);
  // Placing the screen in the document first lets it load.
  css.render(scene, camera);
  await screen.start();

  // The glow behind the phone and the rim lights that colour its edges.
  const halo = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1),
    new THREE.MeshBasicMaterial({
      map: softDisc(256, 1.4), color: glow(P.glow_color, 1.6 * P.glow), transparent: true, blending: THREE.AdditiveBlending, depthWrite: false,
    }),
  );
  halo.scale.set(WIDTH * 3.2, HEIGHT * 1.9, 1);
  halo.position.z = -1.2;
  halo.visible = P.glow > 0;
  scene.add(halo);
  const rimLeft = new THREE.DirectionalLight(color(P.glow_color), 3 * Math.max(0.3, P.glow));
  rimLeft.position.set(-4, 1.5, -3);
  const rimRight = new THREE.DirectionalLight(color(P.glow_color), 2 * Math.max(0.3, P.glow));
  rimRight.position.set(4, -1, -3);
  const key = new THREE.DirectionalLight(0xffffff, 1.4);
  key.position.set(-2, 3, 5);
  scene.add(rimLeft, rimRight, key);

  // A soft backdrop that is lighter behind the phone.
  if (!P.transparent) {
    const backdrop = new THREE.Mesh(
      new THREE.PlaneGeometry(1, 1),
      new THREE.MeshBasicMaterial({ map: softDisc(256, 2), color: color(P.background).lerp(color(P.glow_color), 0.18).multiplyScalar(1.6), transparent: true, depthWrite: false }),
    );
    backdrop.scale.set(14, 14, 1);
    backdrop.position.z = -6;
    scene.add(backdrop);
  }

  const fill = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const fitTall = (HEIGHT / 0.68) / 2 / fill;
  const fitWide = (WIDTH / 0.62) / 2 / (fill * camera.aspect);
  const distance = Math.max(fitTall, fitWide);
  const forward = new THREE.Vector3();

  const pose = (t) => {
    const rise = ease.outCubic(progress(t, P.rise_at, P.rise_duration));
    const turn = ease.inOutCubic(progress(t, P.turn_at, P.turn_duration));
    const settle = ease.outCubic(progress(t, P.rise_at, P.rise_duration + P.turn_duration));
    const up = P.rise_at + P.rise_duration;
    const hover = P.float * Math.max(0, Math.min(1, t - up)) ;
    phone.position.set(0, mix(-HEIGHT * 1.6, 0, rise) + 0.03 * hover * Math.sin((t - up) * 1.9), 0);
    phone.rotation.set(
      THREE.MathUtils.degToRad(P.tilt) * (1 - settle) + 0.025 * hover * Math.sin((t - up) * 1.3),
      THREE.MathUtils.degToRad(P.start_angle) * (1 - turn) + 0.04 * hover * Math.sin((t - up) * 1.1),
      THREE.MathUtils.degToRad(-6) * (1 - settle),
    );
    const push = ease.inOutSine(progress(t, P.push_at, P.push_duration));
    camera.position.set(0, 0.08, distance * (1 - P.push * push));
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    phone.updateMatrixWorld();
    stage.focus.setFromMatrixPosition(phone.matrixWorld);
    // The screen's glass catches the light as the phone turns.
    forward.set(0, 0, 1).applyQuaternion(phone.quaternion);
    const shine = clamp(0.5 + forward.x * 2.2);
    screen.sheen.style.background = `linear-gradient(115deg, rgba(255,255,255,0) ${mix(-30, 70, shine)}%, rgba(255,255,255,0.13) ${mix(-10, 90, shine)}%, rgba(255,255,255,0) ${mix(10, 110, shine)}%)`;
  };

  return async (t) => {
    await screen.show(Math.max(0, P.screen_start + t));
    stage.render(t, pose);
    css.render(scene, camera);
  };
});
