// Plan 17's leaf-textmate-grammar AC1/AC2: a dependency-free (node:fs
// only, no npm install — this sandbox has no npm/yarn/pnpm on PATH)
// tokenizer that applies emerald.tmLanguage.json's own patterns, in
// their own declared precedence order, against the real examples/*.em
// corpus. Not a full oniguruma/TextMate interpreter — a repo-local
// sanity check that the patterns actually fire on real Emerald source
// with zero unmatched residue, and that every keyword this project's
// grammar.lalrpop actually reserves gets a keyword scope somewhere in
// that corpus.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// Recursively lists every `.em` file under `dir` — `examples/packages/`
// (plan 46's real two-package `require` worked example) is the only
// place `require` appears anywhere in this repo's example corpus, so
// a non-recursive `examples/*.em` scan would never exercise it.
function listEmFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      out.push(...listEmFiles(full));
    } else if (entry.endsWith(".em")) {
      out.push(full);
    }
  }
  return out;
}

const here = dirname(fileURLToPath(import.meta.url));
const grammar = JSON.parse(
  readFileSync(join(here, "syntaxes", "emerald.tmLanguage.json"), "utf8"),
);

// Flatten the repository into an ordered list of {name, regex} rules,
// in the same precedence order as the grammar's own top-level
// `patterns` array — first match at a position wins, mirroring
// TextMate's own array-order precedence. `strings` is handled
// specially below (its content can itself contain `#{...}`
// interpolation) rather than forced into this flat list.
const rules = [];
function addPattern(p) {
  if (p.match) {
    rules.push({ name: p.name, regex: new RegExp(p.match, "y") });
  } else if (p.patterns) {
    for (const sub of p.patterns) addPattern(sub);
  }
}
for (const top of grammar.patterns) {
  if (top.include === "#strings") continue;
  addPattern(grammar.repository[top.include.slice(1)]);
}

function tokenizeLine(line) {
  const tokens = [];
  let pos = 0;
  while (pos < line.length) {
    if (/\s/.test(line[pos])) {
      pos++;
      continue;
    }
    if (line[pos] === '"') {
      // A whole string literal: "..." with \" \n escapes and #{...}
      // interpolation (bracket-depth-tracked, not just "scan to the
      // next unescaped quote" — grammar.lalrpop's own interpolation
      // spans a full nested Expr, which can itself contain a string).
      const start = pos;
      pos++;
      let depth = 0;
      while (pos < line.length) {
        if (depth === 0 && line[pos] === '"') {
          pos++;
          break;
        }
        if (line[pos] === "\\" && pos + 1 < line.length) {
          pos += 2;
          continue;
        }
        if (depth === 0 && line.startsWith("#{", pos)) {
          depth++;
          pos += 2;
          continue;
        }
        if (depth > 0 && line[pos] === "}") {
          depth--;
          pos++;
          continue;
        }
        pos++;
      }
      tokens.push({ name: "string.quoted.double.emerald", text: line.slice(start, pos) });
      continue;
    }
    let matched = false;
    for (const rule of rules) {
      rule.regex.lastIndex = pos;
      const m = rule.regex.exec(line);
      if (m && m.index === pos && m[0].length > 0) {
        tokens.push({ name: rule.name, text: m[0] });
        pos += m[0].length;
        matched = true;
        break;
      }
    }
    if (!matched) {
      tokens.push({ name: null, text: line[pos] });
      pos++;
    }
  }
  return tokens;
}

function fail(message) {
  console.error(`FAIL: ${message}`);
  process.exit(1);
}

// grammar.lalrpop's actual literal keyword terminals (verified against
// the real file this session — see the tmLanguage grammar's own
// per-rule comments for exactly where each one is reserved).
const KEYWORDS = [
  "class", "module", "def", "end", "if", "else", "elsif", "unless",
  "while", "until", "for", "in", "return", "break", "next", "puts",
  "raise", "yield", "begin", "rescue", "ensure", "retry", "case",
  "when", "require", "interface", "implements", "read", "new",
  "Array", "Hash", "Proc", "true", "false", "nil",
];

const examplesDir = join(here, "..", "..", "examples");
const files = listEmFiles(examplesDir);
if (files.length === 0) {
  fail("no examples/**/*.em files found");
}

const seenKeywords = new Set();

for (const file of files) {
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n");
  lines.forEach((line, i) => {
    for (const t of tokenizeLine(line)) {
      if (t.name === null) {
        fail(`unmatched residue in ${file}:${i + 1}: ${JSON.stringify(t.text)} (line: ${JSON.stringify(line)})`);
      }
      if (
        t.name === "keyword.control.emerald" ||
        t.name === "keyword.other.emerald" ||
        t.name === "constant.language.emerald" ||
        (t.name === "support.type.emerald" && ["Array", "Hash", "Proc"].includes(t.text))
      ) {
        seenKeywords.add(t.text);
      }
    }
  });
}

const missing = KEYWORDS.filter((k) => !seenKeywords.has(k));
if (missing.length > 0) {
  fail(`these grammar.lalrpop keywords never appeared (with a keyword scope) across examples/*.em: ${missing.join(", ")}`);
}

// AC2 spot check: classes.em's instance-variable-assignment line gets
// distinct instance-variable / type / numeric-literal scopes.
const classesSrc = readFileSync(join(examplesDir, "classes.em"), "utf8");
const classesLines = classesSrc.split("\n");

const ivarLine = classesLines.find((l) => /@\w+\s*=/.test(l));
if (!ivarLine) {
  fail("examples/classes.em has no '@ivar = ...' line to spot-check");
}
if (!tokenizeLine(ivarLine).some((t) => t.name === "variable.other.readwrite.instance.emerald")) {
  fail(`no instance-variable scope found on: ${ivarLine}`);
}

const typeLine = classesLines.find((l) => /:\s*(Int64|Float64)\b/.test(l));
if (typeLine && !tokenizeLine(typeLine).some((t) => t.name === "support.type.emerald")) {
  fail(`no type scope found on: ${typeLine}`);
}

const numLine = classesLines.find((l) => /\b\d+(\.\d+)?\b/.test(l));
if (
  numLine &&
  !tokenizeLine(numLine).some((t) => t.name && t.name.startsWith("constant.numeric"))
) {
  fail(`no numeric-literal scope found on: ${numLine}`);
}

console.log("PASS");
