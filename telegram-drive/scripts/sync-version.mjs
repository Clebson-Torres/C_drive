import fs from "node:fs";
import path from "node:path";

const repoRoot = process.cwd();
const inputTag = process.argv[2] || process.env.RELEASE_TAG || process.env.GITHUB_REF_NAME || "";

if (!inputTag) {
  console.error("sync-version: missing release tag. Provide vX.Y.Z as arg or RELEASE_TAG/GITHUB_REF_NAME.");
  process.exit(1);
}

const version = inputTag.replace(/^refs\/tags\//, "").replace(/^v/, "");

if (!/^\d+\.\d+\.\d+([-.][0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`sync-version: invalid version derived from tag '${inputTag}'`);
  process.exit(1);
}

const files = [
  path.join(repoRoot, "Cargo.toml"),
  path.join(repoRoot, "tauri.conf.json"),
  path.join(repoRoot, "src-tauri", "tauri.conf.json"),
];

for (const file of files) {
  const source = fs.readFileSync(file, "utf8");
  let updated = source;

  if (file.endsWith(".toml")) {
    updated = source.replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
  } else {
    updated = source.replace(/"version"\s*:\s*"[^"]+"/, `"version": "${version}"`);
  }

  if (updated === source && !source.includes(version)) {
    console.error(`sync-version: failed to update version in ${file}`);
    process.exit(1);
  }

  if (updated !== source) {
    fs.writeFileSync(file, updated, "utf8");
  }

  console.log(`sync-version: ${path.relative(repoRoot, file)} -> ${version}`);
}
