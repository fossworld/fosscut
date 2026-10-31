// Every user-visible text of the interface goes through `t` (ADR-0031): this reads each
// component and fails on text written straight into its markup, in a visible attribute
// (title, aria-label, placeholder, alt, label), or as a notice's `text`.

import ts from "typescript";
import { describe, expect, it } from "vitest";

/**
 * Every component and module of the interface but tests, by path, as text. The stand-in
 * engines (engine/) and test data (test/) are left out: they hold project content and
 * engine messages, which are shown as written.
 */
const SOURCES = import.meta.glob(["../**/*.ts", "../**/*.tsx", "!../**/*.test.ts", "!../engine/**", "!../test/**"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** Texts that are the same in every language. */
const SAME_EVERYWHERE = new Set(["FossCut", "LUFS", "dBFS", "LUT", "fosscut mcp", "x", "%", "s", "px", "°"]);

/** Attributes people read. */
const VISIBLE = new Set(["title", "aria-label", "placeholder", "alt", "label", "aria-description"]);

/** True for a text that says something in a language. */
function wordy(text: string): boolean {
  const trimmed = text.replace(/\s+/g, " ").trim();
  if (/^#[0-9a-f]{3,8}$/i.test(trimmed)) return false;
  return /\p{L}{2,}/u.test(trimmed) && !SAME_EVERYWHERE.has(trimmed);
}

/** The literal texts an expression shows: its strings, and the branches of conditions. */
function shown(node: ts.Node): ts.Node[] {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return [node];
  if (ts.isTemplateExpression(node)) return [node];
  if (ts.isParenthesizedExpression(node)) return shown(node.expression);
  if (ts.isConditionalExpression(node)) return [...shown(node.whenTrue), ...shown(node.whenFalse)];
  if (ts.isBinaryExpression(node) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(node.operatorToken.kind)) {
    return shown(node.right);
  }
  return [];
}

function literalText(node: ts.Node): string {
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text;
  if (ts.isTemplateExpression(node)) return [node.head.text, ...node.templateSpans.map((s) => s.literal.text)].join(" ");
  if (ts.isJsxText(node)) return node.text;
  return "";
}

/** Texts written straight into the markup or notices of `file`. */
function untranslated(file: string, text: string): string[] {
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const found: string[] = [];
  const report = (node: ts.Node) => {
    const said = literalText(node);
    if (!wordy(said)) return;
    const { line } = source.getLineAndCharacterOfPosition(node.getStart());
    found.push(`${file.replace(/^\.\.\//, "")}:${line + 1}: ${said.replace(/\s+/g, " ").trim()}`);
  };
  const visit = (node: ts.Node) => {
    if (ts.isJsxText(node)) report(node);
    if (ts.isJsxExpression(node) && node.expression && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) {
      shown(node.expression).forEach(report);
    }
    if (ts.isJsxAttribute(node) && VISIBLE.has(node.name.getText(source)) && node.initializer) {
      if (ts.isStringLiteral(node.initializer)) report(node.initializer);
      else if (ts.isJsxExpression(node.initializer) && node.initializer.expression) shown(node.initializer.expression).forEach(report);
    }
    // Notices: `{ kind: "info", text: "..." }`.
    if (ts.isPropertyAssignment(node) && node.name.getText(source) === "text") shown(node.initializer).forEach(report);
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

describe("translation coverage", () => {
  it("leaves no text in the components that does not go through t", () => {
    expect(Object.keys(SOURCES).length, "the sources were read").toBeGreaterThan(40);
    const found = Object.entries(SOURCES).flatMap(([file, text]) => untranslated(file, text));
    expect(found).toEqual([]);
  });
});
