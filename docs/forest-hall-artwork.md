# Forest hall artwork and video integration

The homepage uses a six-second **Veo 3.1 Standard** forest-hall video, delivered in 4K on desktop and 1080p on mobile. A young adult man and woman in their twenties converse with four seated robots and a floating orb around an amber fire. Moss, ferns, vines, vaulted arches and subtle teal inlays preserve the approved deep forest composition.

The generated scene's fireflies glowed more steadily than requested. A local postproduction pass adds 64 green insects to the actual exported video, with independent wandering paths, staggered glow phases and different pulse lengths. Each fades fully dark and continues moving while unlit. This replaces the previous browser particle layer and uses no additional paid generation. The underlying AI-generated lights remain part of the source footage.

## Generation provenance

- Creative work began September 10, 2026; the paid render completed September 11 UTC (September 10 in the user's timezone).
- The built-in image generator revised the approved hall to add exactly two adults while retaining all five agents.
- Revised first/last frame: `forest-hall-humans-v2.png`, 1672 × 941. SHA-256: `652ab7615b3d62d6d1a39bd83f60277951deec0f456ae60b970d9783dcf9ee86`.
- Video provider/model: Google Gemini API, `veo-3.1-generate-preview`, one eight-second 16:9 4K output using the same supplied first and last frame.
- Downloaded original: `veo-standard-4k-attempt-3-original.mp4`, 16,818,715 bytes, 3840 × 2160, 24 fps, 192 frames, eight seconds, with audio. This is provider 4K output; the supplied still was lower resolution.
- The original still, downloaded video, operation receipt and review sheets are retained unchanged in ignored `.ui-review/video/quality/`.
- [Exact image and video prompts](forest-hall-prompts.md) are tracked separately.

The user approved a dedicated API project/key linked to existing billing and a maximum US$25 including taxes, with at most four renders and a stop once satisfactory. One API render completed successfully. Two earlier AI Studio submissions cleared without a result or operation identifier; their billing outcome is unconfirmed. No further render was submitted after the successful result was reviewed.

At the inspected [Veo pricing](https://ai.google.dev/gemini-api/docs/pricing#veo-3.1), Standard 4K is US$0.60 per output second: US$4.80 before tax for the completed eight-second render. US$14.40 before tax remains conservatively reserved locally including both uncertain UI submissions; this is **not a confirmed invoice**. An additional MX$300 monthly project cap was configured. No recurring Flow subscription was purchased. The temporary local API credential was removed after delivery; the user's dedicated project and key remain in their account.

## Delivered media

All paths below are relative to `site/assets/forest/`.

| Asset | Dimensions | Size |
| --- | --- | ---: |
| `agent-hall-veo-v2.mp4` | 3840 × 2160 | 18,677,378 bytes |
| `agent-hall-veo-small-v2.mp4` | 1920 × 1080 | 4,544,638 bytes |
| `agent-hall-loop-poster-v2.webp` | 1920 × 1080 | 494,716 bytes |
| `agent-hall-loop-poster-small-v2.webp` | 960 × 540 | 161,910 bytes |

Both clips are six seconds, 180 frames, 30 fps, H.264 High, yuv420p, fast-start MP4 with **no audio stream**. Posters come from the delivered first frame. The larger desktop file is an intentional quality tradeoff requested by the user. Existing static `agent-hall-v1.webp` and its small derivative remain in the lower homepage's forest CTA and edge foliage. Superseded 720p video and posters are removed from delivery and remain recoverable in Git/local review.

## Reproducible export

Requires FFmpeg with libx264, Python 3 with NumPy, and ImageMagick:

```sh
bash scripts/export-forest-loop.sh \
  .ui-review/video/quality/veo-standard-4k-attempt-3-original.mp4 \
  .ui-review/video/quality/reexport
```

Set `FFMPEG` to an explicit executable if it is outside `PATH`. The script preserves the input and refuses to overwrite existing output files.

The source is eight seconds at 24 fps. FFmpeg rotates the cycle to begin at frame 12, keeps frames 12–179, and blends frames 180–191 into frames 0–11. Those 180 frames are played at 30 fps to produce exactly six seconds, with a 0.4-second join. This modestly speeds the restrained source motion; it does not synthesize intermediate frames.

`scripts/render-forest-fireflies.py` adds the deterministic firefly pass to the raw RGB24 frame stream before encoding. Smooth closed paths and periodic glow envelopes maintain position and brightness continuity at the loop seam. Large foreground lights stay away from faces and the headline. This is local video compositing, not additional Veo-generated insects or browser animation. The desktop encode uses CRF 18; its Lanczos 1080p derivative uses CRF 19.

## Playback and accessibility

The separate `forest-hall.js` presentation adapter starts the video muted and inline, retaining the matching poster until playback starts. It chooses the 1080p clip at widths up to 700 pixels and 4K above that. The choice happens once per page playback session so resizing does not restart the scene or request another large clip.

Reduced motion, data-saving mode and a saved manual pause prevent the initial video request. Hidden tabs and offscreen scenes pause playback. A live change to reduced motion returns to the poster. Autoplay refusal offers manual Play; failed media requests retain the poster and leave posting available. Manual pause persists locally and now pauses the fireflies inside the same video.

The hall retains space below the task input so the gathering remains visible. The hero stays dark and readable within Light mode while the rest of the page follows the selected theme. Authentication, wallet and draft logic are unchanged.

## Verification

Core preflight, shared navigation synchronization, the site asset gate and `git diff --check` pass. `scripts/test-forest-ui.cjs` passes at 390, 768, 1280 and 1440 pixels, capturing the poster and actual video alongside theme, posting, account and leaderboard states. It checks responsive video resolution, six-second duration, time progression and loop wrap, persistent/offscreen pause, reduced motion, data-saving preferences, refused autoplay with manual recovery, and a failed media request followed by successful task handoff. Account and marketplace services use isolated fixtures.

The delivered 4K video fully decodes with 180 frames and no audio. Source and loop contact sheets were reviewed for framing, people/robot continuity and the join. Browser screenshots were reviewed at the four target widths. The asset gate enforces 20 MB desktop video, 5 MB mobile video, 500 KB desktop poster and 200 KB small poster limits. No wallet transaction or production deployment was performed.

## Earlier iterations

The initial forest still was generated from the site's earlier night-hall artwork. Two free Flow attempts used 30 of the opening 50 credits: a rejected Veo Lite render followed by an accepted Veo Fast 720p render. That six-second delivery and its later independent CSS fireflies were superseded by the current paid Standard 4K scene and baked firefly pass. Their exact historical prompts and provenance remain in preceding Git revisions.
