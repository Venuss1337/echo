/**
 * Collects the license texts of everything ECHO's own build links or bundles:
 * the Rust crates compiled into the Windows executable, the JavaScript
 * packages bundled into the renderer, and the fonts and other assets shipped
 * in `public/`. The native decoder has its own collector
 * (`native/windows-x64/collect-notices.ps1`) because its inputs come from the
 * pinned MSYS2 build, not from these lockfiles.
 *
 * Usage: bun scripts/collect-app-notices.ts <fresh output directory>
 *
 * Writes `THIRD_PARTY_NOTICES.txt` (human-readable, every license text in
 * full) and `third-party-inventory.json` (machine-readable component list).
 * The result is derived only from `Cargo.lock`, `bun.lock` and the files on
 * disk, so it can be regenerated for any release commit.
 */
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";

const output = process.argv[2];
if (!output) throw new Error("Pass a fresh output directory.");
const out = resolve(output);
if (existsSync(out) && readdirSync(out).length)
  throw new Error("The notices output directory must be empty.");
mkdirSync(out, { recursive: true });

/** Files that carry license grants, copyright or attribution notices. */
const licenseFile =
  /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;

interface Component {
  ecosystem: "cargo" | "npm" | "asset";
  name: string;
  version: string;
  license: string;
  source: string;
  licenseFiles: { name: string; text: string }[];
}
const components: Component[] = [];

function readLicenseFiles(directory: string, extra: string[] = []) {
  const names = new Set(
    readdirSync(directory).filter((name) => licenseFile.test(name)),
  );
  for (const name of extra) names.add(name);
  return [...names]
    .sort()
    .filter((name) => existsSync(join(directory, name)))
    .map((name) => ({
      name,
      text: readFileSync(join(directory, name), "utf8"),
    }));
}

// Rust: the normal (non-build, non-dev) dependency closure of the Windows
// x64 executable. Procedural macros run in the compiler and are not linked.
const metadata = spawnSync(
  "cargo",
  [
    "metadata",
    "--format-version",
    "1",
    "--locked",
    "--filter-platform",
    "x86_64-pc-windows-msvc",
    "--manifest-path",
    "src-tauri/Cargo.toml",
  ],
  { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
);
if (metadata.status !== 0) throw new Error(metadata.stderr);
interface CargoPackage {
  id: string;
  name: string;
  version: string;
  license: string | null;
  license_file: string | null;
  repository: string | null;
  manifest_path: string;
  source: string | null;
  targets: { kind: string[] }[];
}
interface CargoNode {
  id: string;
  deps: { pkg: string; dep_kinds: { kind: string | null }[] }[];
}
const cargo = JSON.parse(metadata.stdout) as {
  packages: CargoPackage[];
  resolve: { root: string; nodes: CargoNode[] };
};
const packages = new Map(cargo.packages.map((entry) => [entry.id, entry]));
const nodes = new Map(cargo.resolve.nodes.map((node) => [node.id, node]));
const linked = new Set<string>();
const pending = [cargo.resolve.root];
while (pending.length) {
  const id = pending.pop()!;
  if (linked.has(id)) continue;
  linked.add(id);
  for (const dependency of nodes.get(id)!.deps) {
    if (!dependency.dep_kinds.some((kind) => kind.kind === null)) continue;
    const target = packages.get(dependency.pkg)!;
    if (target.targets.some((item) => item.kind.includes("proc-macro")))
      continue;
    pending.push(dependency.pkg);
  }
}
for (const id of linked) {
  const entry = packages.get(id)!;
  if (id === cargo.resolve.root) continue;
  if (!entry.source?.startsWith("registry+"))
    throw new Error(`Unexpected non-registry crate ${entry.name}`);
  const directory = dirname(entry.manifest_path);
  // A few crates publish without their license file. Their verbatim upstream
  // texts are kept in notices/crates (see SOURCES.txt there).
  const supplement = join("notices/crates", `${entry.name}-${entry.version}`);
  const files = readLicenseFiles(
    directory,
    entry.license_file ? [entry.license_file] : [],
  );
  components.push({
    ecosystem: "cargo",
    name: entry.name,
    version: entry.version,
    license: entry.license ?? "SEE LICENSE FILE",
    source:
      entry.repository ??
      `https://crates.io/crates/${entry.name}/${entry.version}`,
    licenseFiles:
      files.length || !existsSync(supplement)
        ? files
        : readLicenseFiles(supplement),
  });
}

// JavaScript: the production dependency closure that Vite bundles into the
// renderer. Development tools are not distributed.
const manifest = JSON.parse(readFileSync("package.json", "utf8")) as {
  dependencies: Record<string, string>;
};
function findPackage(from: string, name: string) {
  for (let base = from; ; base = dirname(base)) {
    const candidate = join(base, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) return candidate;
    if (dirname(base) === base)
      throw new Error(`Cannot resolve the bundled package ${name}`);
  }
}
const visited = new Set<string>();
const queue = Object.keys(manifest.dependencies).map((name) =>
  findPackage(process.cwd(), name),
);
while (queue.length) {
  const directory = queue.pop()!;
  const info = JSON.parse(
    readFileSync(join(directory, "package.json"), "utf8"),
  ) as {
    name: string;
    version: string;
    license?: string;
    repository?: string | { url: string };
    dependencies?: Record<string, string>;
  };
  const key = `${info.name}@${info.version}`;
  if (visited.has(key)) continue;
  visited.add(key);
  const repository =
    typeof info.repository === "string"
      ? info.repository
      : info.repository?.url;
  components.push({
    ecosystem: "npm",
    name: info.name,
    version: info.version,
    license: info.license ?? "SEE LICENSE FILE",
    source: repository ?? `https://www.npmjs.com/package/${info.name}`,
    licenseFiles: readLicenseFiles(directory),
  });
  for (const name of Object.keys(info.dependencies ?? {}))
    queue.push(findPackage(directory, name));
}

// Assets shipped with the renderer. Each must carry its license beside it.
const assets: Omit<Component, "licenseFiles" | "ecosystem">[] = [
  {
    name: "DM Sans",
    version: "public/fonts/DMSans.ttf",
    license: "OFL-1.1",
    source: "https://github.com/googlefonts/dm-fonts",
  },
  {
    name: "Instrument Serif",
    version: "public/fonts/InstrumentSerif.ttf",
    license: "OFL-1.1",
    source: "https://github.com/Instrument/instrument-serif",
  },
];
for (const asset of assets) {
  const font = asset.version.replace(/\.ttf$/, "-LICENSE.txt");
  components.push({
    ...asset,
    ecosystem: "asset",
    licenseFiles: [
      { name: font.split("/").pop()!, text: readFileSync(font, "utf8") },
    ],
  });
}
components.push({
  ecosystem: "asset",
  name: "create-tauri-app template",
  version: "notices/create-tauri-app-MIT.txt",
  license: "MIT",
  source: "https://github.com/tauri-apps/create-tauri-app",
  licenseFiles: [
    {
      name: "create-tauri-app-MIT.txt",
      text: readFileSync("notices/create-tauri-app-MIT.txt", "utf8"),
    },
  ],
});

components.sort(
  (a, b) =>
    a.ecosystem.localeCompare(b.ecosystem) ||
    a.name.localeCompare(b.name) ||
    a.version.localeCompare(b.version),
);

// Every distributed component must carry its license text. A new dependency
// without one fails here until its upstream text is added to notices/crates.
const missing = components.filter((entry) => !entry.licenseFiles.length);
if (missing.length)
  throw new Error(
    `No license text for ${missing.map((entry) => `${entry.name} ${entry.version}`).join(", ")}. Add the exact upstream file to notices/crates/<name>-<version>/ and record it in SOURCES.txt.`,
  );

const rule = "=".repeat(78);
const sections = [
  "ECHO — third-party notices for the application and its renderer",
  "",
  "ECHO itself is licensed under the GNU General Public License, version 3 or",
  "(at your option) any later version. See LICENSE.",
  "",
  "This file lists the Rust crates linked into the ECHO executable, the",
  "JavaScript packages bundled into its interface and the fonts it ships,",
  "with every license text found in each component's published package.",
  "The native audio decoder (libmpv, FFmpeg and their dependencies) carries",
  "its own notices under notices/ in the installation directory and in the",
  "native source archive published with each release.",
  "",
  `Components: ${components.length}.`,
  "",
];
for (const entry of components) {
  sections.push(
    rule,
    `${entry.name} ${entry.version} (${entry.ecosystem})`,
    `License: ${entry.license}`,
    `Source: ${entry.source}`,
    rule,
    "",
  );
  for (const file of entry.licenseFiles)
    sections.push(`--- ${file.name} ---`, file.text.trimEnd(), "");
}
writeFileSync(join(out, "THIRD_PARTY_NOTICES.txt"), sections.join("\n"));
writeFileSync(
  join(out, "third-party-inventory.json"),
  JSON.stringify(
    components.map(({ licenseFiles, ...entry }) => ({
      ...entry,
      licenseFiles: licenseFiles.map((file) => file.name),
    })),
    null,
    2,
  ) + "\n",
);
console.log(`Collected license texts for ${components.length} components.`);
