#!/usr/bin/env node
// `@gglint/cli` on npm: runs the gglint program built for this machine, which npm installs from
// the optional dependency `@gglint/cli-<platform>-<arch>`. No install script downloads it,
// so it works where install scripts don't run, as with pnpm and Bun.
"use strict";

const { spawnSync } = require("child_process");

const name = `@gglint/cli-${process.platform}-${process.arch}`;
const program = process.platform === "win32" ? "gglint.exe" : "gglint";

let binary;
try {
  binary = require.resolve(`${name}/bin/${program}`);
} catch {
  console.error(
    `gglint: ${name} isn't installed. gglint is built for macOS, Linux and Windows, on x64 and ` +
      "arm64, and npm installs the build for this machine as an optional dependency: install " +
      "again without --omit=optional or --no-optional. On another machine, run `uvx gglint`.",
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit", windowsHide: true });
if (result.error) {
  console.error(`gglint: couldn't run ${binary}: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
