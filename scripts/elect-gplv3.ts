/**
 * Applies ECHO's GNU LGPL 2.1 section 3 election to one extracted source
 * tree: every notice that applies the GNU Lesser (or Library) General Public
 * License is changed to apply the GNU General Public License, version 3 or
 * later, as section 3 permits for a copy. License texts themselves are left
 * untouched; the GPL text accompanies the result.
 *
 *   bun scripts/elect-gplv3.ts <extracted source> <output directory> <name>
 *
 * Writes `<name>-gplv3-election.patch` (apply with `patch -p1` in the
 * pristine tree), `<name>-elected-files.txt` and
 * `<name>-remaining-references.txt`, which lists files that still mention
 * the LGPL after conversion (for example a license text, or a file also
 * offered under another license) so a person can confirm each one.
 */
import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

const [source, output, name] = process.argv.slice(2);
if (!source || !output || !/^[a-z0-9.+-]+$/i.test(name ?? ""))
  throw new Error("Usage: elect-gplv3.ts <source> <output> <name>");
const root = resolve(source);
const out = resolve(output);
mkdirSync(out, { recursive: true });

/** Files that are license documents, not notices applying a license. */
const licenseText =
  /^(copying|licen[cs]e|lgpl|gpl)([-._].*)?$|^copying\.lib$|^copying\.lgplv?2(\.1)?$/i;
const mentions = /Lesser[\s*#;!/-]*General|Library[\s*#;!/-]*General|LGPL/;

function* walk(directory: string): Generator<string> {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === ".git") continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) yield* walk(path);
    else if (entry.isFile()) yield path;
  }
}

/**
 * Rewrites one file's notices. Text is handled as Latin-1 so every byte
 * round-trips unchanged except the replaced words.
 */
function elect(text: string) {
  // Comment decoration that may sit between words wrapped across lines.
  const gap = String.raw`(\s*(?:(?:\*|#|;|!|//|--|dnl|@c|@|%|')[ \t]*)*\s*)`;
  let next = text.replace(
    new RegExp(
      String.raw`GNU${gap}(?:Lesser|Library)${gap}General${gap}Public`,
      "g",
    ),
    (_, a: string, _b: string, c: string) => `GNU${a}General${c}Public`,
  );
  // Version references that follow a converted notice, such as
  // "either version 2.1 of the License" or "version 2 of the License".
  next = next.replace(
    new RegExp(
      String.raw`(General${gap}Public${gap}License[\s\S]{0,400}?version${gap})2(?:\.1)?(?=${gap}(?:of|,|\.|or))`,
      "g",
    ),
    (_, lead: string) => `${lead}3`,
  );
  // Machine-readable identifiers on SPDX lines.
  next = next.replace(
    /(SPDX-License-Identifier:[^\n]*?)\bLGPL-(?:2\.0|2\.1|3\.0)(?:-or-later|-only|\+)?/g,
    "$1GPL-3.0-or-later",
  );
  // Short forms in prose and build metadata ("LGPL v2.1+", "LGPL2.1+",
  // "LGPLv2.1 (or later)"), only on lines that state license terms, so a
  // historical remark such as "relicensed from GPL to LGPL" stays true.
  next = next.replace(/^.*$/gm, (line) =>
    /\b(under|terms|license:)/i.test(line) && !/relicens/i.test(line)
      ? line.replace(
          /\bLGPL(?:[ -]?v?(2\.0|2\.1|2|3)(\+|-or-later)?)?(?![-\w]|\.\d)/g,
          (match, number?: string, later?: string) =>
            number ? (later ? "GPLv3+" : "GPLv3") : "GPL",
        )
      : line,
  );
  // In a file whose notice was converted, the remaining fragments belong to
  // that notice even when code splits it, as in generators that print the
  // notice into the tables they write.
  if (next !== text)
    next = next
      .replaceAll("Lesser General Public License", "General Public License")
      .replaceAll("version 2.1 of the License", "version 3 of the License");
  return next;
}

const elected: string[] = [];
const remaining: string[] = [];
const stage = join(out, `.${name}-stage`);
for (const path of walk(root)) {
  const rel = relative(root, path).replaceAll("\\", "/");
  if (statSync(path).size > 4 * 1024 * 1024) continue;
  const bytes = readFileSync(path);
  if (bytes.subarray(0, 8192).includes(0)) continue;
  const text = bytes.toString("latin1");
  if (!mentions.test(text)) continue;
  const base = rel.split("/").pop()!;
  const document =
    licenseText.test(base) ||
    /^\s*GNU (LESSER|LIBRARY) GENERAL PUBLIC LICENSE/.test(text.slice(0, 400));
  if (document) {
    remaining.push(`${rel}\tlicense text, kept verbatim`);
    continue;
  }
  // In source code, only comments are notices. Lines holding string
  // literals (a library's version text, a CLI that prints the license of
  // another configuration, a generator's template) are masked so compiled
  // output stays byte-identical; they are reported as remaining references.
  const masked: string[] = [];
  const code = /\.(c|h|cc|cpp|cxx|hh|hpp|s|asm|inc|m|mm)$/i.test(base);
  const guarded = code
    ? text.replace(/^.*".*$/gm, (line) => {
        masked.push(line);
        return `\u0000${masked.length - 1}\u0000`;
      })
    : text;
  const next = elect(guarded).replace(
    /\u0000(\d+)\u0000/g,
    (_, index: string) => masked[Number(index)],
  );
  if (next !== text) {
    elected.push(rel);
    for (const [side, content] of [
      ["a", text],
      ["b", next],
    ] as const) {
      const target = join(stage, side, rel);
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, Buffer.from(content, "latin1"));
    }
  }
  const left = next.split(/\r?\n/).find((line) => mentions.test(line));
  if (left) remaining.push(`${rel}\t${left.trim().slice(0, 160)}`);
}

let patch = "";
if (elected.length) {
  const diff = spawnSync(
    "git",
    // Byte-exact output: no line-ending conversion from local Git settings.
    [
      "-c",
      "core.autocrlf=false",
      "-c",
      "core.safecrlf=false",
      "diff",
      "--no-index",
      "--no-color",
      "--text",
      "--src-prefix=a/",
      "--dst-prefix=b/",
      "a",
      "b",
    ],
    { cwd: stage, encoding: "latin1", maxBuffer: 1024 * 1024 * 1024 },
  );
  if (diff.status !== 1) throw new Error(diff.stderr || "git diff failed");
  // git names paths a/a/<file> and b/b/<file>; strip the stage directories.
  patch = diff.stdout
    .replaceAll(/^(diff --git a\/)a\/(\S+) b\/b\//gm, "$1$2 b/")
    .replaceAll(/^--- a\/a\//gm, "--- a/")
    .replaceAll(/^\+\+\+ b\/b\//gm, "+++ b/");
}
spawnSync(
  process.platform === "win32" ? "cmd" : "rm",
  process.platform === "win32"
    ? ["/c", "rmdir", "/s", "/q", stage]
    : ["-rf", stage],
);
writeFileSync(
  join(out, `${name}-gplv3-election.patch`),
  Buffer.from(patch, "latin1"),
);
writeFileSync(
  join(out, `${name}-elected-files.txt`),
  elected.sort().join("\n") + "\n",
);
writeFileSync(
  join(out, `${name}-remaining-references.txt`),
  remaining.sort().join("\n") + "\n",
);
console.log(
  `${name}: ${elected.length} files elected, ${remaining.length} remaining references`,
);
