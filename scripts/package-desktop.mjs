import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = join(root, "src-tauri", "target", "release", "unidl.exe");
const bundleDir = join(root, "src-tauri", "target", "release", "bundle");
const outputDir = join(root, "release");
const output = join(outputDir, "UniDL.exe");

if (!existsSync(source)) {
  throw new Error("Missing desktop executable. Run npm run build:desktop first.");
}

if (existsSync(bundleDir)) {
  throw new Error("Installer bundle output exists. UniDL release packaging is exe-only.");
}

mkdirSync(outputDir, { recursive: true });
copyFileSync(source, output);

console.log(`Packaged ${output}`);
