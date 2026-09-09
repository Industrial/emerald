// Plan 17's leaf-vscode-extension AC2: verifies package.json's
// contributes.languages/grammars actually agree with the real grammar
// file leaf-textmate-grammar produced, rather than trusting the two
// files to stay in sync by inspection alone. Dependency-free — only
// node:fs and node:path, no npm install needed (see plan's Decision
// log: no npm/yarn/pnpm on PATH in this sandbox).

import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));

function fail(message) {
  console.error(`FAIL: ${message}`);
  process.exit(1);
}

const languages = pkg.contributes?.languages;
if (!Array.isArray(languages) || languages.length === 0) {
  fail("package.json contributes.languages is missing or empty");
}

const lang = languages[0];
if (lang.id !== "emerald") {
  fail(`contributes.languages[0].id is ${JSON.stringify(lang.id)}, expected "emerald"`);
}
if (!Array.isArray(lang.extensions) || !lang.extensions.includes(".em")) {
  fail(`contributes.languages[0].extensions does not include ".em": ${JSON.stringify(lang.extensions)}`);
}

const grammars = pkg.contributes?.grammars;
if (!Array.isArray(grammars) || grammars.length === 0) {
  fail("package.json contributes.grammars is missing or empty");
}

const grammar = grammars[0];
if (grammar.scopeName !== "source.emerald") {
  fail(`contributes.grammars[0].scopeName is ${JSON.stringify(grammar.scopeName)}, expected "source.emerald"`);
}

const grammarPath = join(here, grammar.path);
if (!existsSync(grammarPath)) {
  fail(`contributes.grammars[0].path (${grammar.path}) does not exist on disk`);
}

const grammarFile = JSON.parse(readFileSync(grammarPath, "utf8"));
if (grammarFile.scopeName !== "source.emerald") {
  fail(
    `the grammar file's own "scopeName" (${JSON.stringify(grammarFile.scopeName)}) does not match ` +
      `package.json's declared scopeName ("source.emerald") — the manifest and the grammar have drifted`,
  );
}

console.log("PASS");
