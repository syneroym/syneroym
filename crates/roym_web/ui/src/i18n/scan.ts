import ts from "typescript";

export interface MessageScan {
  /// The literal id of every `t("…")` and `messageId("…")` call.
  ids: string[];
  /// A use the scan cannot follow, with its line.
  problems: string[];
}

const MARKERS = new Set(["t", "messageId"]);

/// Finds the message ids a source file uses. It parses the source, so a
/// `//` inside a string and a `t("x")` inside a comment are told apart
/// from code.
export function findMessageIds(source: string, fileName = "source.ts"): MessageScan {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ES2022, true);
  const scan: MessageScan = { ids: [], problems: [] };
  const where = (node: ts.Node): string =>
    `${fileName}:${file.getLineAndCharacterOfPosition(node.getStart(file)).line + 1}`;

  const visit = (node: ts.Node): void => {
    if (ts.isImportSpecifier(node) && node.propertyName && MARKERS.has(node.propertyName.text)) {
      scan.problems.push(`${where(node)}: ${node.propertyName.text} is imported under another name`);
    }
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && MARKERS.has(node.expression.text)) {
      const first = node.arguments[0];
      if (first && (ts.isStringLiteral(first) || ts.isNoSubstitutionTemplateLiteral(first))) {
        scan.ids.push(first.text);
      } else {
        scan.problems.push(`${where(node)}: ${node.expression.text}() needs a string literal id`);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return scan;
}
