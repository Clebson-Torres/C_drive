import { readdirSync, readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("../src/ui-app/", import.meta.url));
const files = [];

function walk(dir) {
  for (const entry of readdirSync(dir)) {
    const absolute = join(dir, entry);
    const stat = statSync(absolute);
    if (stat.isDirectory()) {
      walk(absolute);
      continue;
    }
    if (!/\.(jsx?|tsx?)$/.test(entry)) continue;
    files.push(absolute);
  }
}

walk(root);

const offenders = [];
for (const file of files) {
  const source = readFileSync(file, "utf8");
  if (source.includes("innerHTML")) {
    offenders.push(file);
  }
}

if (offenders.length) {
  console.error("innerHTML is forbidden in src/ui-app:");
  offenders.forEach((file) => console.error(` - ${file}`));
  process.exit(1);
}

console.log("No innerHTML usage found in src/ui-app.");
