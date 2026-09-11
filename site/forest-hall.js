/* Decorative media only. Account, draft and financial logic remain separate. */
(() => {
  const scene = document.querySelector("[data-forest-scene]");
  const control = document.querySelector("[data-forest-pause]");
  if (!scene || !control) return;
  const video = scene.querySelector("[data-forest-video]");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const connection = navigator.connection;
  const preferenceKey = "agent-bounties.forest-paused";
  let paused = false, visible = true, failed = false;
  try { paused = localStorage.getItem(preferenceKey) === "true"; } catch (_) { /* Optional preference. */ }
  // The firefly pass is baked into the video alongside the Veo animation.
  // Keep the matching poster visible until the locally hosted Veo clip plays.
  // Motion and data-saving preferences are checked before requesting video.
  function update() {
    const constrained = reduced.matches || Boolean(connection?.saveData);
    const running = !paused && !constrained && visible && !document.hidden;
    scene.dataset.forestMotion = running ? "running" : "paused";
    control.hidden = constrained;
    control.textContent = paused ? "Play atmosphere" : "Pause atmosphere";
    control.setAttribute("aria-label", control.textContent);
    control.setAttribute("aria-pressed", String(paused));
    if (!video || !video.dataset.src || failed) return;
    if (!running) {
      video.pause();
      if (constrained) { video.hidden = true; scene.dataset.mediaState = "artwork"; }
      return;
    }
    if (!video.getAttribute("src")) {
      // Choose once when playback is requested; resizing must not restart a
      // playing clip or download a second large file. Preference checks above
      // still prevent both sources from loading for reduced motion/data saving.
      video.src = video.dataset.srcSmall && matchMedia("(max-width: 700px)").matches
        ? video.dataset.srcSmall : video.dataset.src;
    }
    video.muted = true;
    video.play().catch(error => {
      // Pausing while playback is starting is expected when visibility or
      // motion preferences change. It must not disable subsequent playback.
      if (error.name === "AbortError") return;
      if (error.name === "NotAllowedError") {
        // A native autoplay refusal leaves a usable manual Play control.
        paused = true; video.hidden = true; scene.dataset.mediaState = "artwork";
        update();
        return;
      }
      failed = true; video.hidden = true; scene.dataset.mediaState = "artwork";
    });
  }
  if (video) {
    video.addEventListener("playing", () => {
      video.hidden = false;
      scene.dataset.mediaState = "video";
      if (scene.dataset.forestMotion !== "running") video.pause();
    });
    video.addEventListener("error", () => { failed = true; video.hidden = true; scene.dataset.mediaState = "artwork"; });
  }
  control.addEventListener("click", () => {
    paused = !paused;
    try { localStorage.setItem(preferenceKey, String(paused)); } catch (_) { /* Works without storage. */ }
    update();
  });
  reduced.addEventListener("change", update);
  connection?.addEventListener("change", update);
  document.addEventListener("visibilitychange", update);
  if ("IntersectionObserver" in window) new IntersectionObserver(entries => {
    visible = entries[0].isIntersecting;
    update();
  }, { threshold: 0 }).observe(scene);
  update();
})();
