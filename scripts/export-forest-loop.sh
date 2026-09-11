#!/usr/bin/env bash
set -euo pipefail
# Input must be the reviewed eight-second 3840x2160, 24 fps Veo source.
# Originals are preserved. Refuse to overwrite existing export files.
source_video="${1:?Provide the reviewed Veo master}"
export_dir="${2:?Provide an export directory}"
video_ffmpeg="${FFMPEG:-ffmpeg}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
for command in "$video_ffmpeg" python3 magick; do
  command -v "$command" >/dev/null || { echo "Missing dependency: $command" >&2; exit 1; }
done
python3 -c 'import numpy'
mkdir -p "$export_dir"
for output in agent-hall-veo-v2.mp4 agent-hall-veo-small-v2.mp4 loop-first-frame.png agent-hall-loop-poster-v2.webp agent-hall-loop-poster-small-v2.webp; do
  if [[ -e "$export_dir/$output" ]]; then
    echo "Refusing to overwrite $export_dir/$output" >&2
    exit 1
  fi
done
"$video_ffmpeg" -hide_banner -i "$source_video" \
  -filter_complex_threads 2 \
  -filter_complex "[0:v]fps=24,split=3[bodyin][tailin][headin];[bodyin]trim=start_frame=12:end_frame=180,setpts=PTS-STARTPTS[body];[tailin]trim=start_frame=180:end_frame=192,setpts=PTS-STARTPTS[tail];[headin]trim=start_frame=0:end_frame=12,setpts=PTS-STARTPTS[head];[tail][head]blend=all_expr='A*(1-min(N/11,1))+B*min(N/11,1)'[join];[body][join]concat=n=2:v=1:a=0,format=rgb24[v]" \
  -map '[v]' -an -frames:v 180 -fps_mode passthrough -f rawvideo - | \
python3 "$script_dir/render-forest-fireflies.py" | \
"$video_ffmpeg" -hide_banner -n -f rawvideo -pixel_format rgb24 -video_size 3840x2160 -framerate 30 -i - \
  -an -c:v libx264 -pix_fmt yuv420p -threads 6 -preset slow -crf 18 -movflags +faststart \
  "$export_dir/agent-hall-veo-v2.mp4"
"$video_ffmpeg" -hide_banner -n -i "$export_dir/agent-hall-veo-v2.mp4" \
  -vf scale=1920:1080:flags=lanczos -an -c:v libx264 -threads 6 -preset slow -crf 19 -movflags +faststart \
  "$export_dir/agent-hall-veo-small-v2.mp4"
"$video_ffmpeg" -hide_banner -n -i "$export_dir/agent-hall-veo-v2.mp4" -frames:v 1 \
  "$export_dir/loop-first-frame.png"
magick "$export_dir/loop-first-frame.png" -resize 1920x1080 -quality 78 \
  "$export_dir/agent-hall-loop-poster-v2.webp"
magick "$export_dir/loop-first-frame.png" -resize 960x540 -quality 80 \
  "$export_dir/agent-hall-loop-poster-small-v2.webp"
