import { Aperture, Move3d, Orbit, Video } from "lucide-react";

import type { Editor } from "../app/editor";
import type { CameraView, ClipView } from "../engine/types";
import { t } from "../i18n";
import { Glyph } from "../ui/controls";
import { KeyedFields } from "./Keyed";
import type { KeyedField } from "./keyedFields";

/** The engine's defaults for what a camera leaves out (ADR-0037). */
export const CAMERA_DEFAULTS = {
  field_of_view: 40,
  position: [0, 0],
  position_z: 0,
  point_of_interest: [0, 0],
  point_of_interest_z: 0,
  orbit: [0, 0],
  roll: 0,
  depth_of_field: false,
  focus_offset: 0,
  aperture: 0.1,
  max_blur: 0.02,
} as const satisfies Required<CameraView>;

/** Every field of the camera, at its stored value or its default. */
export function cameraOf(clip: ClipView): Required<CameraView> {
  const c = clip.camera ?? {};
  const d = CAMERA_DEFAULTS;
  return {
    field_of_view: c.field_of_view ?? d.field_of_view,
    position: c.position ?? [...d.position],
    position_z: c.position_z ?? d.position_z,
    point_of_interest: c.point_of_interest ?? [...d.point_of_interest],
    point_of_interest_z: c.point_of_interest_z ?? d.point_of_interest_z,
    orbit: c.orbit ?? [...d.orbit],
    roll: c.roll ?? d.roll,
    depth_of_field: c.depth_of_field ?? d.depth_of_field,
    focus_offset: c.focus_offset ?? d.focus_offset,
    aperture: c.aperture ?? d.aperture,
    max_blur: c.max_blur ?? d.max_blur,
  };
}

/** The `camera.set` call that gives `field` (a camera field's name) `value`, sparse: a default removes it. */
export function cameraSet(clip: string, field: keyof CameraView, value: unknown): { command: string; args: unknown } {
  const fallback = CAMERA_DEFAULTS[field];
  const same = JSON.stringify(value) === JSON.stringify(fallback);
  return { command: "camera.set", args: { clip, set: { [field]: same ? null : value } } };
}

/** The camera's fields by group, as keyed numbers: paths `camera.<field>`. */
export function cameraFields(camera: Required<CameraView>): Record<"lens" | "place" | "target" | "orbit" | "focus", KeyedField[]> {
  const field = (name: keyof CameraView, extra: Omit<KeyedField, "path" | "plain">, part?: 0 | 1): KeyedField => ({
    path: `camera.${name}`,
    part,
    plain: camera[name] as number | [number, number],
    ...extra,
  });
  return {
    lens: [
      field("field_of_view", { label: t("space.camera.fov"), unit: "°", min: 1, max: 170, range: [10, 120], help: t("space.camera.fov.help") }),
      field("roll", { label: t("space.camera.roll"), unit: "°", range: [-180, 180], help: t("space.camera.roll.help") }),
    ],
    place: [
      field("position", { label: t("space.camera.x"), unit: "%", scale: 100, range: [-100, 100] }, 0),
      field("position", { label: t("space.camera.y"), unit: "%", scale: 100, range: [-100, 100] }, 1),
      field("position_z", { label: t("space.camera.dolly"), unit: "%", scale: 100, range: [-200, 100], help: t("space.camera.dolly.help") }),
    ],
    target: [
      field("point_of_interest", { label: t("space.camera.x"), unit: "%", scale: 100, range: [-100, 100] }, 0),
      field("point_of_interest", { label: t("space.camera.y"), unit: "%", scale: 100, range: [-100, 100] }, 1),
      field("point_of_interest_z", { label: t("space.camera.z"), unit: "%", scale: 100, range: [-100, 300] }),
    ],
    orbit: [
      field("orbit", { label: t("space.camera.yaw"), unit: "°", range: [-180, 180], help: t("space.camera.yaw.help") }, 0),
      field("orbit", { label: t("space.camera.pitch"), unit: "°", range: [-89, 89], help: t("space.camera.pitch.help") }, 1),
    ],
    focus: [
      field("focus_offset", { label: t("space.camera.focus"), unit: "%", scale: 100, range: [-100, 200], help: t("space.camera.focus.help") }),
      field("aperture", { label: t("space.camera.aperture"), unit: "%", scale: 100, min: 0, max: 100, range: [0, 100], help: t("space.camera.aperture.help") }),
      field("max_blur", { label: t("space.camera.blur"), unit: "%", scale: 100, min: 0, max: 20, range: [0, 20], step: 0.1, help: t("space.camera.blur.help") }),
    ],
  };
}

/**
 * A camera clip's inspector (ADR-0037): its lens, where it is, what it looks at, how it
 * turns around that point, and its depth of field. Every number takes keyframes; clips with
 * 3D on are seen through the camera while it covers them.
 */
export function Camera({ editor, clip, playhead }: { editor: Editor; clip: ClipView; playhead: number }) {
  const camera = cameraOf(clip);
  const fields = cameraFields(camera);
  const plain = (field: KeyedField, value: number | [number, number]) =>
    cameraSet(clip.id, field.path.slice("camera.".length) as keyof CameraView, value);
  const group = (key: keyof typeof fields) => <KeyedFields editor={editor} clip={clip} playhead={playhead} fields={fields[key]} plain={plain} />;
  return (
    <>
      <section className="effects camera-panel">
        <h2>
          <span>{t("space.camera.lens")}</span>
          <Glyph icon={Video} size="sm" />
        </h2>
        {group("lens")}
        <p className="hint">{t("space.camera.help")}</p>
      </section>
      <section className="effects">
        <h2>
          <span>{t("space.camera.position")}</span>
          <Glyph icon={Move3d} size="sm" />
        </h2>
        {group("place")}
        <h3>{t("space.camera.target")}</h3>
        {group("target")}
      </section>
      <section className="effects">
        <h2>
          <span>{t("space.camera.orbit")}</span>
          <Glyph icon={Orbit} size="sm" />
        </h2>
        {group("orbit")}
      </section>
      <section className="effects">
        <h2>
          <span>{t("space.camera.dof")}</span>
          <Glyph icon={Aperture} size="sm" />
        </h2>
        <label className="toggle" title={t("space.camera.dof.help")}>
          <input
            type="checkbox"
            checked={camera.depth_of_field}
            onChange={(event) => void editor.edit([cameraSet(clip.id, "depth_of_field", event.target.checked)])}
          />
          <span>{t("space.camera.dof.on")}</span>
        </label>
        {camera.depth_of_field && group("focus")}
      </section>
    </>
  );
}
