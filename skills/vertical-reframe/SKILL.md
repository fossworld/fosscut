---
name: vertical-reframe
description: Turn a 16:9 FossCut edit into a 9:16 (or 1:1) version where each shot keeps its subject in frame, without black bars, leaving the original edit untouched. Use when the user asks for a vertical, portrait, Reels, Shorts or TikTok version of a video, or to reframe or crop it for another aspect ratio.
---

# Vertical reframe

Follow the safe editing loop of the `fosscut-basics` skill (read, plan with a dry run, apply with `expected_revision` and a `request_id`, check, export). This skill adds the reframing steps.

## What a good result looks like

- A new timeline at the target size (1080x1920 for 9:16, 1080x1080 for 1:1). The original wide timeline is unchanged.
- No black bars: every picture covers the canvas.
- In every shot, the subject stays well inside the frame: a face, a speaker, the product. The crop moves smoothly and only when the subject does.
- Captions sit inside the vertical safe area.

## Choose the path

- **People on screen, and the local models installed** (`mediapipe-pose-full`, `onnxruntime`; `models` lists them, and installing needs the user's yes): the automatic path follows each person.
- **A product, an animal or anything else:** follow the subject picked by its region. The `matte` analysis with model `subject` also works, but takes seconds a frame.

## Automatic path

1. **Read** the wide timeline with `timeline_read` (`detail: "full"`): its video clips and their assets.
2. **Measure** each video asset once with `media_analyze`: `analyzers: ["pose", "scenes"]` (or `["matte", "scenes"]` with `options: {"matte": {"model": "subject", "every": 5}}`). Follow running jobs with `job_status`.
3. **Make the copy.** `auto_edit` with action `vertical`, `width` and `height`, and `name`. It duplicates the timeline onto the new canvas, and frames every video clip shot by shot at the scene cuts. Do a `dry_run` first and tell the user how many clips it frames.
4. **Note** the new timeline id (`vertical.timeline`) and pass it as `timeline` to every later read, frame and export. Clips listed in `vertical.centred` had no subject; frame them by region (below) or leave them centred.

## Region path

1. **Copy onto the new canvas.** `edit` with `timeline.duplicate`, for example `{"width": 1080, "height": 1920, "fit": "fill", "name": "Vertical"}`. Note the new timeline id.
2. **Split at the shots.** On the copy, `auto_edit` with action `split_scenes` for each video clip, then read the copy for the shot clips' ids.
3. **Find the subject** in each shot: a `frame_grab` of the wide timeline at the shot's start, and judge its centre and size as fractions of the picture (`x`, `y`, `width`, `height`, 0 to 1).
4. **Follow it.** `auto_edit` with action `reframe`, the shot's `clip`, `region`, `at` (the time of that frame) and `shots: false`. The crop keeps the region in frame as it moves; where it is lost, the crop holds.

## Check and deliver

1. **Captions.** If the wide edit had captions, the copy has them; check their look on the new canvas and adjust it with `captions.style` and `{"placement": ...}`. Or caption the copy with the `viral-captions` skill.
2. **Check.** `contact_sheet` of the new timeline: no bars, every subject in frame. `frame_grab` in shots that were hard to frame. Show the user, and redo shots they do not like with a region, or with `framing` (`max_zoom` to allow zooming in, `dead_zone` and `max_speed` for a calmer camera).
3. **Export** the new timeline: `export` with its `timeline`. Report the shots and how each was framed.

## When something fails

- `fosscut.analysis.missing`: run the analysis named in the message with `media_analyze`, then retry.
- `fosscut.reframe.no_subject`: nobody was found in the clip. Follow a region instead.
- A missing model: tell the user its size and license from `models` and install it only if they agree; otherwise use the region path.
- **Black bars:** the timeline was copied without `fill`. Copy again with `fill`.
- `fosscut.revision.conflict`: read the new timeline again and re-plan.
