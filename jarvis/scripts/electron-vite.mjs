// Runs electron-vite with ELECTRON_RUN_AS_NODE removed.
// VS Code (and tools launched from it) set ELECTRON_RUN_AS_NODE=1, which makes electron.exe behave
// like plain Node, so `require("electron").app` is undefined and the app crashes on start.
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

delete process.env.ELECTRON_RUN_AS_NODE;

// The package's "exports" map hides bin/, so resolve the file path directly.
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cli = path.join(root, "node_modules", "electron-vite", "bin", "electron-vite.js");
const child = spawn(process.execPath, [cli, ...process.argv.slice(2)], { stdio: "inherit", env: process.env });
child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 0)));
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => child.kill(sig));
