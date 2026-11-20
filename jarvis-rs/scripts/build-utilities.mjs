import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const require = createRequire(resolve(root, "../jarvis/package.json"));
const esbuild = require("esbuild");
await esbuild.build({
  entryPoints: [resolve(root, "scripts/utilities.ts")],
  outfile: resolve(root, "target/utilities.cjs"),
  platform: "node", target: "node24", format: "cjs", bundle: true,
  plugins: [{
    name: "jarvis-utility-settings",
    setup(build) {
      build.onResolve({ filter: /^\.\.\/config$/ }, args => {
        if (dirname(args.importer).endsWith("tools")) return { path: resolve(root, "scripts/utility-settings.ts") };
      });
    },
  }],
});
