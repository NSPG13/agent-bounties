"use strict";
const assert = require("node:assert/strict");

// Measure the loaded font's ink, not CSS line boxes: display fonts can extend
// outside their line boxes even when the page itself has no horizontal scroll.
module.exports = async function assertHeadingLayout(page, label) {
  await page.evaluate(() => document.fonts.ready);
  const failures = await page.evaluate(() => {
    if (document.body.classList.contains("forest-home")) return [];
    const canvas = document.createElement("canvas").getContext("2d"), failures = [];
    for (const heading of document.querySelectorAll("h1, h2, h3")) {
      if (!heading.checkVisibility()) continue;
      const lines = [], walker = document.createTreeWalker(heading, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        const node = walker.currentNode;
        if (!node.textContent.trim()) continue;
        const css = getComputedStyle(node.parentElement);
        canvas.font = `${css.fontStyle} ${css.fontWeight} ${css.fontSize} ${css.fontFamily}`;
        const ink = canvas.measureText(node.textContent), range = document.createRange();
        range.selectNodeContents(node);
        for (const rect of range.getClientRects()) {
          const baseline = rect.top + ink.fontBoundingBoxAscent;
          const top = baseline - ink.actualBoundingBoxAscent, bottom = baseline + ink.actualBoundingBoxDescent;
          const line = lines.find(line => Math.abs(line.baseline - baseline) < 2);
          if (line) { line.top = Math.min(line.top, top); line.bottom = Math.max(line.bottom, bottom); }
          else lines.push({ baseline, top, bottom });
          if (rect.left < -1 || rect.right > innerWidth + 1) failures.push(`${heading.textContent.trim()}: text outside viewport`);
        }
      }
      lines.sort((a, b) => a.baseline - b.baseline);
      for (let i = 1; i < lines.length; i++) {
        if (lines[i].top < lines[i - 1].bottom - 1) failures.push(`${heading.textContent.trim()}: overlapping text lines`);
      }
    }
    return failures;
  });
  assert.deepEqual(failures, [], `Readable headings: ${label}`);
};
