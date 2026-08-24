import { build } from "esbuild";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const outdir = process.env.COINBASE_WALLET_OUTDIR
  ? path.resolve(process.env.COINBASE_WALLET_OUTDIR)
  : path.join(root, "target", "coinbase-embedded-wallet");
const outfile = path.join(outdir, "coinbase-embedded-wallet.bundle.js");
const cssfile = path.join(outdir, "coinbase-embedded-wallet.bundle.css");
const checkOnly = process.argv.includes("--check");

await mkdir(path.dirname(outfile), { recursive: true });
await build({
  entryPoints: [path.join(here, "src/index.js")],
  outfile,
  bundle: true,
  format: "iife",
  platform: "browser",
  target: ["es2022"],
  minify: true,
  sourcemap: false,
  legalComments: "none",
  define: {
    "process.env.NODE_ENV": '"production"',
  },
  banner: {
    js: "/* Agent Bounties Coinbase embedded-wallet adapter. Generated; do not edit directly. */",
  },
});

await access(outfile);
const bundle = await readFile(outfile, "utf8");
for (const marker of [
  "coinbase-embedded",
  "eth_requestAccounts",
  "Agent Bounties embedded wallet",
  "createOnLogin",
  "Use the same sign-in method",
  "Link another sign-in method",
  "authMethodLinking",
]) {
  if (!bundle.includes(marker)) throw new Error(`Built Coinbase wallet bundle is missing ${marker}`);
}
if (bundle.includes("CDP_API_KEY_SECRET") || bundle.includes("CDP_WALLET_SECRET")) {
  throw new Error("Server-side Coinbase secrets must not enter the browser bundle.");
}
try {
  await access(cssfile);
} catch (_error) {
  await writeFile(cssfile, "/* Coinbase CDP component styles are bundled in JavaScript for this release. */\n", "utf8");
}
if (checkOnly) {
  await writeFile(path.join(here, ".build-check"), "ok\n", "utf8");
}
console.log(`Built ${path.relative(root, outfile)} (${Buffer.byteLength(bundle)} bytes)`);
