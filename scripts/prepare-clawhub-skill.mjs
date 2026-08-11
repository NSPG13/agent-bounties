import { access, cp, lstat, mkdir, readdir } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const CLAWHUB_SKILL_SOURCE_ENTRIES = Object.freeze([
  "SKILL.md",
  "README.md",
  "LICENSE",
  "fixtures",
  "references",
  "scripts",
]);

const defaultSourceDir = fileURLToPath(
  new URL("../skills/agent-bounties/", import.meta.url),
);

function isInside(parent, candidate) {
  const path = relative(parent, candidate);
  return path !== "" && !path.startsWith("..") && !isAbsolute(path);
}

async function inventoryRegularFiles(root) {
  const files = [];

  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const absolutePath = join(directory, entry.name);
      const relativePath = relative(root, absolutePath).replaceAll("\\", "/");
      if (entry.isSymbolicLink()) {
        throw new Error(`ClawHub staging refuses symbolic link: ${relativePath}`);
      }
      if (entry.isDirectory()) {
        await walk(absolutePath);
      } else if (entry.isFile()) {
        const fileStat = await lstat(absolutePath);
        files.push({ path: relativePath, size: fileStat.size });
      }
    }
  }

  await walk(root);
  return files.sort((left, right) => left.path.localeCompare(right.path));
}

export async function prepareClawHubSkill({
  sourceDir = defaultSourceDir,
  outputDir,
} = {}) {
  if (!outputDir) throw new Error("--output is required");

  const source = resolve(sourceDir);
  const output = resolve(outputDir);
  if (source === output || isInside(source, output)) {
    throw new Error("ClawHub staging output must be outside the canonical skill directory");
  }

  await access(join(source, "SKILL.md"));
  try {
    await access(output);
    throw new Error(`ClawHub staging output already exists: ${output}`);
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }

  await mkdir(dirname(output), { recursive: true });
  await mkdir(output, { recursive: false });
  for (const entry of CLAWHUB_SKILL_SOURCE_ENTRIES) {
    await cp(join(source, entry), join(output, entry), {
      recursive: true,
      errorOnExist: true,
      force: false,
    });
  }

  const files = await inventoryRegularFiles(output);
  if (!files.some((file) => file.path === "SKILL.md")) {
    throw new Error("ClawHub staging output is missing SKILL.md");
  }

  return {
    schema_version: "agent-bounties/clawhub-skill-stage-v1",
    source,
    output,
    source_entries: [...CLAWHUB_SKILL_SOURCE_ENTRIES],
    excluded_source_entries: [".claude-plugin"],
    file_count: files.length,
    total_bytes: files.reduce((total, file) => total + file.size, 0),
    files,
  };
}

function parseArguments(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--output") {
      parsed.outputDir = argv[index + 1];
      index += 1;
    } else if (argument === "--source") {
      parsed.sourceDir = argv[index + 1];
      index += 1;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }
  return parsed;
}

async function main() {
  const result = await prepareClawHubSkill(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
