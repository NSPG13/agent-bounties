import { build } from "esbuild";
import { readFile, mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { gzipSync } from "node:zlib";
import { storageRecoveryPlugin } from "./storage-recovery-plugin.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = process.env.PHONE_WALLET_REPO_ROOT || path.resolve(here, "../..");
const outfile = path.join(root, "site/vendor/phone-wallet.bundle.js");
const result = await build({
  entryPoints: [path.join(here, "src/index.js")], outfile,
  bundle: true, write: false, format: "esm", platform: "browser", target: "es2022",
  minify: true, sourcemap: false, legalComments: "eof", metafile: true,
  define: { "process.env.NODE_ENV": '"production"' },
  // This adapter always supplies its own QR UI. Exclude the unused AppKit
  // modal and its exchange/embedded-wallet adapters from the shipped code.
  plugins: [storageRecoveryPlugin, { name: "no-appkit-modal", setup(builder) {
    builder.onResolve({ filter: /^@reown\/appkit\/core$/ }, () => ({ path: "appkit-disabled", namespace: "phone-wallet" }));
    builder.onLoad({ filter: /.*/, namespace: "phone-wallet" }, () => ({ contents: 'export function createAppKit() { throw new Error("Agent Bounties uses its own phone-wallet review."); }' }));
  } }],
});
if (!Object.keys(result.metafile.inputs).some(name => name.replaceAll("\\", "/").endsWith("src/reconnecting-store.js"))) throw new Error("Phone wallet storage recovery is missing from the bundle.");
const generated = result.outputFiles[0].text;
const licenseStart = generated.indexOf("/*! Bundled license information:");
if (licenseStart < 0) throw new Error("Expected bundled license notices are missing.");
// Normalize whitespace only in esbuild's trailing license comment, never in
// executable code or template strings from dependencies.
const bytes = Buffer.from(generated.slice(0, licenseStart) + generated.slice(licenseStart).replace(/[ \t]+$/gm, ""));
if (gzipSync(bytes).length > 200000) throw new Error("Phone wallet lazy bundle exceeds its 200 kB gzip budget.");
if (Object.keys(result.metafile.inputs).some((name) => /node_modules\/(?:@coinbase|@reown\/appkit\/|axios\/)/.test(name))) throw new Error("Unrelated wallet or exchange adapters entered the phone bundle.");
await mkdir(path.dirname(outfile), { recursive: true });
if (process.argv.includes("--check")) {
  if (Buffer.from(bytes).toString("utf8") !== (await readFile(outfile, "utf8")).replace(/\r\n/g, "\n")) throw new Error("Rebuild and commit the phone-wallet bundle.");
} else await writeFile(outfile, bytes);
console.log(`Phone wallet bundle verified: ${bytes.length} bytes, ${gzipSync(bytes).length} bytes gzip.`);
