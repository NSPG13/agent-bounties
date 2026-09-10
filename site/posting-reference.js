(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingReference = api;
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const VERSION = 1;
  const EXTENSION = "x-agent-bounties-reference-attachment";
  const REVISION = "933c9c446a76d26f148a4f2defacf6453d02a7b2";
  const SOURCE_URL = "https://agentbounties.app/";
  const ASSET_ROOT = `https://raw.githubusercontent.com/NSPG13/agent-bounties/${REVISION}/site/`;
  const MAX_BYTES = 384 * 1024;
  const TIMEOUT_MS = 8000;
  // These byte hashes pin the existing homepage originals at REVISION. Update
  // the revision and all eight entries together when the homepage art changes.
  const MANIFEST = Object.freeze({
    "dawn-desktop": ["a7cdaf198c5ed18cf4921d5d5a180353d7f461f96906822cfc3489e229d0c29a", 202068],
    "dawn-mobile": ["ded24e4a5aaa0593caf7411ba998bc8e36dea6cb6a5042b69762035645858629", 96120],
    "day-desktop": ["f9143ee70ca0551bc97562c89c96cc56b4a54391ab4034315148253b757fcaef", 256228],
    "day-mobile": ["7a62291ef8cafe5ce7371a55c79e44432cfe338c8e884d98c95b3fd1a4a172cd", 98768],
    "dusk-desktop": ["73e5628a4184a854654df37cc6cf46d18ba14a5f45962619a6606c5627bb24a9", 255156],
    "dusk-mobile": ["6a478e020ba6362ce8899d761d858e4dd6ba35221bc89fc76a078bd911bad5ad", 113114],
    "night-desktop": ["06fc595b47101033af0ac00a71b4052f31c84d924cc1078e37a42e855d7883fa", 149004],
    "night-mobile": ["9705a9ccb07c91ff4cdb360fc6dc6393c5fb7f149bb7479877b9bef4dfcc8b9c", 61982],
  });
  function asset(phase, variant) {
    const entry = MANIFEST[`${phase}-${variant}`];
    if (!entry) throw new Error("Choose one of the homepage background images and its desktop or mobile version.");
    const path = `assets/solarpunk/scene-${phase}${variant === "mobile" ? "-mobile" : ""}.webp`;
    return { path, asset_url: `${ASSET_ROOT}${path}`, sha256: `sha256:${entry[0]}`, byte_length: entry[1] };
  }
  function suggestedPhase(date = new Date()) {
    const minute = date.getHours() * 60 + date.getMinutes() + date.getSeconds() / 60;
    // Dominant plate, including the tie rules in SolarpunkHome.sceneBlend.
    return minute <= 300 ? "night" : minute <= 450 ? "dawn" : minute <= 1065 ? "day" : minute < 1155 ? "dusk" : "night";
  }
  function validate(value) {
    if (value == null) return null;
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("The frozen reference is invalid.");
    const expected = asset(value.phase, value.variant);
    if (value.version !== VERSION || value.kind !== "homepage_background" || value.source_url !== SOURCE_URL
      || value.asset_url !== expected.asset_url || value.sha256 !== expected.sha256
      || value.byte_length !== expected.byte_length || value.mime_type !== "image/webp"
      || typeof value.captured_at !== "string" || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,3})?(?:Z|[+-]\d\d:\d\d)$/.test(value.captured_at)
      || !Number.isFinite(Date.parse(value.captured_at)) || Date.parse(value.captured_at) > Date.now() + 300000) {
      throw new Error("The frozen reference does not match the supported immutable image, digest and capture time. Capture it again before approving.");
    }
    const clean = { version: VERSION, kind: "homepage_background", source_url: SOURCE_URL, asset_url: expected.asset_url,
      captured_at: value.captured_at, sha256: expected.sha256, mime_type: "image/webp", byte_length: expected.byte_length,
      phase: value.phase, variant: value.variant };
    if (Object.keys(value).some((key) => !Object.hasOwn(clean, key))) throw new Error("The frozen reference contains unsupported fields.");
    return Object.freeze(clean);
  }
  async function boundedBytes(response) {
    const length = response.headers.get("content-length");
    if (length && (!/^\d+$/.test(length) || Number(length) > MAX_BYTES)) throw new Error("The background image exceeds the reference size limit.");
    if (!response.body?.getReader) throw new Error("This browser cannot safely read the reference image. Use a current browser.");
    const reader = response.body.getReader(), chunks = [];
    let size = 0;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > MAX_BYTES) { await reader.cancel(); throw new Error("The background image exceeds the reference size limit."); }
        chunks.push(value);
      }
    } finally { reader.releaseLock(); }
    if (!size) throw new Error("The reference image is empty; nothing was captured.");
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return bytes;
  }
  async function captureHomepage({ phase, variant }, win = globalThis) {
    const expected = asset(phase, variant);
    if (!win.crypto?.subtle) throw new Error("A secure browser connection is required to verify the reference image.");
    const url = new URL(expected.path, win.location.href);
    if (url.origin !== win.location.origin) throw new Error("The reference must come from this site.");
    const controller = new win.AbortController();
    const timeout = win.setTimeout(() => controller.abort(), TIMEOUT_MS);
    try {
      const response = await win.fetch(url.href, { cache: "no-store", credentials: "omit", redirect: "error", referrerPolicy: "no-referrer", signal: controller.signal });
      if (!response.ok) throw new Error(`The background image could not be read (${response.status}); no reference was captured.`);
      if (response.redirected || (response.url && new URL(response.url).origin !== url.origin)) throw new Error("The reference image redirected to another location.");
      if (response.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "image/webp") throw new Error("The background response was not a WebP image.");
      const bytes = await boundedBytes(response);
      const digest = await win.crypto.subtle.digest("SHA-256", bytes);
      const sha256 = `sha256:${Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
      if (sha256 !== expected.sha256 || bytes.byteLength !== expected.byte_length) {
        throw new Error("The homepage image has changed since its saved reference version. No reference was attached; the site must refresh its image version before you can freeze this background.");
      }
      return validate({ version: VERSION, kind: "homepage_background", source_url: SOURCE_URL, asset_url: expected.asset_url,
        captured_at: new Date().toISOString(), sha256, mime_type: "image/webp", byte_length: bytes.byteLength, phase, variant });
    } catch (error) {
      if (controller.signal.aborted) throw new Error("The background image could not be verified within 8 seconds. Nothing was captured; retry when your connection is ready.");
      throw error;
    } finally { win.clearTimeout(timeout); }
  }
  function withEvidence(schema, reference) {
    if (!reference) return schema;
    if (!schema || typeof schema !== "object" || Array.isArray(schema)) throw new Error("Prepare the evidence schema before binding a reference image.");
    const attachment = validate(reference);
    if (schema[EXTENSION] && JSON.stringify(validate(schema[EXTENSION])) !== JSON.stringify(attachment)) throw new Error("The evidence schema binds a different reference image. Review a revised draft before replacing it.");
    return { ...schema, [EXTENSION]: attachment };
  }
  function mount(container, options = {}) {
    if (!container) return null;
    const win = options.window || window, doc = container.ownerDocument;
    const make = (tag, text) => { const node = doc.createElement(tag); if (text) node.textContent = text; return node; };
    const heading = make("strong", "Freeze a homepage background");
    const help = make("p", "Choose the image your solver should use. This freezes the background asset, without the homepage text, moving effects or time-of-day blend.");
    const controls = make("div"), phase = make("select"), variant = make("select");
    phase.setAttribute("aria-label", "Homepage background scene");
    variant.setAttribute("aria-label", "Background layout");
    for (const name of ["dawn", "day", "dusk", "night"]) { const option = make("option", name[0].toUpperCase() + name.slice(1)); option.value = name; phase.append(option); }
    for (const name of ["desktop", "mobile"]) { const option = make("option", name === "desktop" ? "Desktop image" : "Mobile image"); option.value = name; variant.append(option); }
    phase.value = suggestedPhase(); variant.value = win.matchMedia?.("(max-width: 720px)").matches ? "mobile" : "desktop";
    const capture = make("button", "Freeze this background"), remove = make("button", "Remove reference");
    for (const button of [capture, remove]) { button.type = "button"; button.className = "button secondary"; }
    const preview = make("img"); preview.alt = "Selected homepage background asset"; preview.width = 320; preview.loading = "lazy";
    const status = make("output"); status.setAttribute("aria-live", "polite");
    const original = make("a", "Open the frozen original"); original.target = "_blank"; original.rel = "noopener noreferrer";
    let saved = null, busy = false;
    function update(reference) {
      try { saved = validate(reference); } catch (error) { saved = null; status.textContent = error.message; }
      if (saved) {
        phase.value = saved.phase; variant.value = saved.variant;
        status.textContent = `Frozen ${new Date(saved.captured_at).toLocaleString()}. SHA-256 ${saved.sha256.slice(7, 19)}… (${saved.byte_length.toLocaleString()} bytes). Included in the public terms when posted.`;
        original.href = saved.asset_url;
      } else if (!reference) status.textContent = "No reference attached. The selection follows your local time; choose the background you intend.";
      preview.src = saved ? saved.asset_url : asset(phase.value, variant.value).path;
      original.hidden = !saved; remove.hidden = !saved;
    }
    function selectionChanged() {
      preview.src = asset(phase.value, variant.value).path;
      status.textContent = saved ? "Preview changed. Your saved reference is unchanged until you freeze this image." : "Preview only. Freeze this image to attach its verified original.";
    }
    phase.addEventListener("change", selectionChanged); variant.addEventListener("change", selectionChanged);
    capture.addEventListener("click", async () => {
      if (busy) return;
      busy = true; capture.disabled = true; remove.disabled = true; phase.disabled = true; variant.disabled = true;
      status.textContent = "Reading and verifying the original image…";
      try { const reference = await captureHomepage({ phase: phase.value, variant: variant.value }, win); await options.onChange?.(reference); update(reference); }
      catch (error) { status.textContent = error.message || "The reference could not be captured. Your saved reference is unchanged."; }
      finally { busy = false; capture.disabled = false; remove.disabled = false; phase.disabled = false; variant.disabled = false; }
    });
    remove.addEventListener("click", async () => { if (busy) return; try { await options.onChange?.(null); update(null); } catch (error) { status.textContent = error.message; } });
    controls.append(phase, variant, capture, remove); container.replaceChildren(heading, help, controls, preview, status, original);
    update(options.getReference?.() || null);
    return { update };
  }
  return { VERSION, EXTENSION, REVISION, MAX_BYTES, TIMEOUT_MS, asset, suggestedPhase, validate, captureHomepage, withEvidence, mount };
});
