import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(scriptDir, "..");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const args = process.argv.slice(2);

if (args.length === 0) {
  console.error("run-npm-command: missing npm arguments");
  process.exit(1);
}

const child = process.platform === "win32"
  ? spawn([npmCommand, ...args].join(" "), {
      cwd: projectRoot,
      stdio: "inherit",
      env: process.env,
      shell: true,
    })
  : spawn(npmCommand, args, {
      cwd: projectRoot,
      stdio: "inherit",
      env: process.env,
    });

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

child.on("error", (error) => {
  console.error(`run-npm-command: failed to start ${npmCommand}: ${error.message}`);
  process.exit(1);
});
