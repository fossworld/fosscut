# Three.js

The 3D library FossCut's scene templates are written with ([ADR-0042](../../../../../../docs/adr/0042-ship-threejs-scene-templates-as-web-compositions.md)). `fosscut-core` embeds these files and writes them into each scene composition's own folder, under `vendor/three/`, so a page loads them by relative paths and nothing from the network. This README is not copied.

| | |
|---|---|
| Library | three.js 0.185.1 by the three.js authors |
| License | MIT ([LICENSE](LICENSE)), copied into every composition folder beside the files |
| Source | npm package `three@0.185.1`, `https://registry.npmjs.org/three/-/three-0.185.1.tgz`, published 2026-07-01 |
| Integrity | `sha512-5aojFCXKwnjBRZvUnt3WFfEcvUJgkN5LlijRFN95hMy8WVkG4I0QNcJE+OuWvuJ0bOdStrbfXn0pkd6/QyiAlg==` (registry and tarball agree; SHA-256 `a2143f5bf978bd3470a51024b2b6bdd581913ba8f36ff1538d433f3a95adf2df`, 5,320,348 bytes) |
| Why this version | The newest release at least two weeks old that still ships minified builds; 0.186.0 and later publish only unminified ones |
| Files taken | `build/three.module.min.js`, `build/three.core.min.js`, `LICENSE`, and the add-ons below from `examples/jsm/` (unmodified, under `addons/` with their relative layout) |
| Added | 2026-11-02 |

| File | Bytes | SHA-256 |
|---|---|---|
| `three.module.min.js` | 365,552 | `86bcee248b64f44bcfc23c331ae74619061957d59cab040171dcb6fb5900beb6` |
| `three.core.min.js` | 385,386 | `05b2609338c76cd65daf74f3ac515bc9a5045e1b3b33edc07d8c9bd55250fa90` |
| `addons/renderers/CSS3DRenderer.js` | 11,126 | `cd1f6bfaff0e2efa8a95b8cca6e96b0eea957d6a6057d875de8bab720b49057b` |
| `addons/loaders/SVGLoader.js` | 76,850 | `a58536582ec55d41d9ed8bedaf0633398cf9004110cb4a9741d572bded4a3129` |
| `addons/loaders/GLTFLoader.js` | 114,959 | `97642d720f16cc9a0c9844934198e4d0c023bea8e89576d0f7545d03b2d103d2` |
| `addons/geometries/RoundedBoxGeometry.js` | 6,476 | `c1b7c9bd2cddff2e3f3a0723f618a3d364a47450e3d25771d21faed88410bec8` |
| `addons/utils/BufferGeometryUtils.js` | 37,621 | `5c552223a9309883743b80538d6e9cdb45e3227f30d3ec56fb2c39b46e78d595` |
| `addons/utils/SkeletonUtils.js` | 11,535 | `b1632a703206c3d830de9fcbe515696770d04b71a15ee6b50afa6d2c3298c86f` |
| `LICENSE` | 1,081 | `8b378ebe60e2fe500158cb0ac71cb5e8b7d92953c2abcc63a0eb90499653b5bc` |

Together 1,010,586 bytes. `GLTFLoader.js` needs the two `utils/` files; the other add-ons need only Three.js. The templates draw bloom, depth of field and motion blur with FossCut's own passes in `presets/library/scene-kit/stage.js`, so no post-processing add-ons are taken.

The MIT license allows copying, changing and redistributing the files in any software, commercial included, as long as the copyright notice and license travel with them; every composition folder carries `LICENSE`. Frames rendered with Three.js carry no obligations.

To update: fetch the package with `npm pack three@<version>`, check its integrity against `npm view three@<version> dist.integrity`, copy the same files, update the tables, and rerun the scene tests and their goldens (`cargo xtask goldens`).
