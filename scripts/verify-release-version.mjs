import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const desktopPackage = JSON.parse(
  await readFile(resolve(root, "apps/desktop/package.json"), "utf8"),
);
const tauriConfig = JSON.parse(
  await readFile(resolve(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
);
const cargoToml = await readFile(
  resolve(root, "apps/desktop/src-tauri/Cargo.toml"),
  "utf8",
);
const cargoVersion = cargoToml.match(
  /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
)?.[1];

const versions = {
  "apps/desktop/package.json": desktopPackage.version,
  "apps/desktop/src-tauri/tauri.conf.json": tauriConfig.version,
  "apps/desktop/src-tauri/Cargo.toml": cargoVersion,
};
const unique = new Set(Object.values(versions));
if (unique.size !== 1 || unique.has(undefined)) {
  console.error("Release versions do not match:", versions);
  process.exit(1);
}

const version = desktopPackage.version;
const tag = process.env.GITHUB_REF_NAME || process.argv[2];
if (tag && tag !== `v${version}`) {
  console.error(`Tag ${tag} does not match application version v${version}.`);
  process.exit(1);
}

console.log(`Release version verified: v${version}`);
