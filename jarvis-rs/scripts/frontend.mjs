import "./build-utilities.mjs";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const workspace = fileURLToPath(new URL("../", import.meta.url));
const electron = resolve(workspace, "../jarvis");
const require = createRequire(resolve(electron, "package.json"));
const vite = resolve(dirname(require.resolve("vite/package.json")), "bin/vite.js");
const child = spawn(process.execPath, [
  vite, "build", resolve(electron, "src/renderer"),
  "--base", "./", "--outDir", resolve(workspace, "dist"), "--emptyOutDir",
], { cwd: electron, stdio: "inherit", windowsHide: true });
child.on("error", error => console.error("Could not build the Jarvis renderer:", error.message));
child.on("close", code => { process.exitCode = code ?? 1; });

