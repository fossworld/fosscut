#!/usr/bin/env node
// Enforce the npm side of the dependency license policy (ADR-0014).
// Usage: node scripts/check-npm-licenses.mjs <package dir>   (run after `pnpm install`)

import { execFileSync } from "node:child_process";

const ALLOWED = new Set([
  "MIT",
  "MIT-0",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "0BSD",
  "Zlib",
  "Unicode-3.0",
  "MPL-2.0",
  "CC0-1.0",
  "CC-BY-4.0",
  "BlueOak-1.0.0",
  "Python-2.0",
]);

function allowed(expression) {
  const expr = expression.replace(/[()]/g, " ").trim();
  if (/\sOR\s/i.test(expr)) {
    return expr.split(/\s+OR\s+/i).some((part) => allowed(part));
  }
  if (/\sAND\s/i.test(expr)) {
    return expr.split(/\s+AND\s+/i).every((part) => allowed(part));
  }
  return ALLOWED.has(expr);
}

const dir = process.argv[2] ?? ".";
const raw = execFileSync("pnpm", ["licenses", "list", "--prod", "--json"], {
  cwd: dir,
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
const byLicense = JSON.parse(raw);

const violations = [];
for (const [license, packages] of Object.entries(byLicense)) {
  if (allowed(license)) continue;
  for (const pkg of packages) {
    violations.push(`${pkg.name}@${(pkg.versions ?? [pkg.version]).join(",")}: ${license}`);
  }
}

if (violations.length > 0) {
  console.error("Dependencies with licenses outside the policy (ADR-0014):");
  for (const line of violations) console.error(`  ${line}`);
  process.exit(1);
}
console.log(`npm license policy OK (${Object.keys(byLicense).length} license kinds)`);
