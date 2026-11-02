// 3D extruded logo, made by FossCut for its library (ADR-0042): the filled shapes of an SVG
// extruded with a bevel, swinging in and settling, with a band of light sweeping across.

import * as THREE from 'three';
import { SVGLoader } from 'three/addons/loaders/SVGLoader.js';
import {
  Stage, clamp, color, compose, ease, loadParams, mix, progress, softDisc, studio,
} from './kit/stage.js';

// fosscut:defaults
const DEFAULTS = {
  "logo": null,
  "face_color": "#F4F6FA",
  "side_color": "#FF7A2F",
  "sweep_color": "#FFFFFF",
  "depth": 0.22,
  "bevel": 0.015,
  "metalness": 0.3,
  "roughness": 0.25,
  "in_at": 0.2,
  "in_duration": 1.3,
  "sweep_at": 1.6,
  "sweep_duration": 1.1,
  "spin": 75,
  "idle": 6,
  "transparent": false,
  "background": "#0A0C12",
  "exposure": 1,
  "tone_mapping": "aces",
  "bloom": { "strength": 1.2, "radius": 0.6, "threshold": 0.9 },
  "depth_of_field": { "aperture": 0.3, "max_blur": 0.01 },
  "motion_blur": { "samples": 8, "shutter": 0.5 }
};
// fosscut:end

/** Adds a band of light that sweeps across `material` in world space. */
const sweepable = (material, sweep) => {
  material.onBeforeCompile = (shader) => {
    shader.uniforms.sweepColor = sweep.color;
    shader.uniforms.sweepAt = sweep.at;
    shader.uniforms.sweepWidth = sweep.width;
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vSweep;')
      .replace('#include <begin_vertex>', '#include <begin_vertex>\nvSweep = (modelMatrix * vec4(transformed, 1.0)).xyz;');
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', '#include <common>\nuniform vec3 sweepColor;\nuniform float sweepAt;\nuniform float sweepWidth;\nvarying vec3 vSweep;')
      .replace('#include <emissivemap_fragment>', `#include <emissivemap_fragment>
        float across = dot(vSweep, normalize(vec3(1.0, 0.55, 0.0)));
        float band = exp(-pow((across - sweepAt) / sweepWidth, 2.0));
        totalEmissiveRadiance += sweepColor * band;`);
  };
  return material;
};

compose(async () => {
  const P = await loadParams(DEFAULTS);
  const stage = new Stage(P, { fov: 28, near: 0.1, far: 80 });
  const { scene, camera, renderer } = stage;
  scene.environment = studio(renderer, { key: 1.3, tint: '#ffffff', rim: P.side_color });
  scene.environmentIntensity = 1;

  // The drawing: the logo slot's SVG, or FossCut's mark beside the page.
  const src = P.logo && P.logo.src ? P.logo.src : 'mark.svg';
  const text = await (await fetch(src)).text();
  const data = new SVGLoader().parse(text);
  const shapes = [];
  for (const path of data.paths) {
    const fill = path.userData && path.userData.style ? path.userData.style.fill : undefined;
    if (fill === 'none') {
      continue;
    }
    shapes.push(...SVGLoader.createShapes(path));
  }
  if (!shapes.length) {
    throw new Error(`the logo ${src} has no filled shapes to extrude`);
  }
  // Sized by the drawing's width, so depth and bevel are fractions of it.
  const flat = new THREE.ShapeGeometry(shapes);
  flat.computeBoundingBox();
  const span = flat.boundingBox.getSize(new THREE.Vector3());
  const unit = Math.max(span.x, span.y, 1e-6);
  flat.dispose();
  const bevel = P.bevel * unit;
  const geometry = new THREE.ExtrudeGeometry(shapes, {
    depth: Math.max(1e-4, P.depth * unit - bevel * 2),
    bevelEnabled: bevel > 0,
    bevelThickness: bevel,
    bevelSize: bevel * 0.8,
    bevelSegments: 6,
    curveSegments: 32,
  });
  geometry.computeBoundingBox();
  const centre = geometry.boundingBox.getCenter(new THREE.Vector3());
  geometry.translate(-centre.x, -centre.y, -centre.z);
  // SVG's y runs down: a half turn about x stands the drawing up with its first face in front.
  geometry.rotateX(Math.PI);
  geometry.scale(1 / unit, 1 / unit, 1 / unit);

  const sweep = {
    color: { value: new THREE.Color() },
    at: { value: -10 },
    width: { value: 0.09 },
  };
  const face = sweepable(new THREE.MeshPhysicalMaterial({
    color: color(P.face_color), metalness: P.metalness, roughness: P.roughness, clearcoat: 0.8, clearcoatRoughness: 0.12,
  }), sweep);
  const side = sweepable(new THREE.MeshPhysicalMaterial({
    color: color(P.side_color), metalness: Math.min(1, P.metalness + 0.25), roughness: Math.max(0.08, P.roughness * 0.8),
  }), sweep);
  const logo = new THREE.Mesh(geometry, [face, side]);
  logo.castShadow = true;
  const holder = new THREE.Group();
  holder.add(logo);
  scene.add(holder);
  const logoWidth = span.x / unit;
  const logoHeight = span.y / unit;

  // Light: a cool key from above left, a warm rim in the side colour, and a soft glow behind.
  const key = new THREE.DirectionalLight(0xffffff, 2.8);
  key.position.set(-3, 4, 5);
  const rim = new THREE.DirectionalLight(color(P.side_color), 2.5);
  rim.position.set(4, 1, -4);
  scene.add(key, rim, new THREE.HemisphereLight(0xe8eeff, 0x1a1410, 0.4));
  if (!P.transparent) {
    const backdrop = new THREE.Mesh(
      new THREE.PlaneGeometry(1, 1),
      new THREE.MeshBasicMaterial({ map: softDisc(256, 2), color: color(P.background).lerp(color(P.side_color), 0.12).multiplyScalar(1.4), transparent: true, depthWrite: false }),
    );
    backdrop.scale.set(9, 9, 1);
    backdrop.position.z = -5;
    scene.add(backdrop);
  }

  const tan = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
  const distance = Math.max(
    (logoWidth / 0.68) / 2 / (tan * camera.aspect),
    (logoHeight / 0.6) / 2 / tan,
  );
  const sweepColor = color(P.sweep_color).multiplyScalar(4);
  const reach = (logoWidth + logoHeight) * 0.5;

  const pose = (t) => {
    const p = progress(t, P.in_at, P.in_duration);
    const swing = ease.outBack(p, 1.2);
    const arrive = ease.outCubic(p);
    const settled = Math.max(0, t - (P.in_at + P.in_duration));
    holder.rotation.set(
      THREE.MathUtils.degToRad(12) * (1 - arrive) + 0.04 * Math.sin(settled * 1.2) * clamp(settled),
      THREE.MathUtils.degToRad(P.spin) * (1 - swing) + THREE.MathUtils.degToRad(P.idle) * settled,
      THREE.MathUtils.degToRad(-8) * (1 - arrive),
    );
    holder.position.set(0, mix(-0.15, 0, arrive), mix(-3, 0, arrive));
    logo.scale.set(1, 1, mix(0.05, 1, ease.outCubic(clamp(p * 1.4))));
    holder.visible = t >= P.in_at;
    const s = progress(t, P.sweep_at, P.sweep_duration);
    sweep.at.value = mix(-reach, reach, ease.inOutSine(s));
    sweep.color.value.copy(sweepColor).multiplyScalar(s > 0 && s < 1 ? 1 : 0);
    camera.position.set(0, 0.04, distance * mix(1.08, 1, ease.outCubic(progress(t, 0, P.in_at + P.in_duration + 2))));
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    stage.focus.set(0, 0, 0);
  };

  return (t) => stage.render(t, pose);
});
