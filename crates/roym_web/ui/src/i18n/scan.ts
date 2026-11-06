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

const LETTER = /\p{L}/u;
const SENTENCE = /\p{L}\s+\p{L}/u;

const TEXT_PROPERTIES = new Set([
  "textContent",
  "innerText",
  "placeholder",
  "title",
  "alt",
  "ariaLabel",
  "label",
]);
const TEXT_ATTRIBUTES = new Set([
  "aria-label",
  "aria-description",
  "aria-placeholder",
  "title",
  "placeholder",
  "alt",
  "label",
]);
const NODE_CALLS = new Set([
  "append",
  "prepend",
  "replaceChildren",
  "before",
  "after",
  "replaceWith",
  "createTextNode",
  "alert",
  "confirm",
  "prompt",
]);
/// Hub helper -> the arguments that are shown text: the first index, and
/// whether every argument after it counts too.
const HELPER_ARGS = new Map<string, { from: number; rest: boolean }>([
  ["text", { from: 1, rest: false }],
  ["field", { from: 0, rest: false }],
  ["labelled", { from: 1, rest: true }],
  ["input", { from: 0, rest: false }],
  ["checkbox", { from: 0, rest: false }],
]);
const TEXT_KEYS = new Set(["label", "text", "title", "placeholder", "hint"]);
const CLASS_ASSIGNMENTS = new Set(["className", "rel"]);
const CLASS_LIST_CALLS = new Set(["add", "remove", "toggle", "replace"]);
const SELECTOR_CALLS = new Set(["querySelector", "querySelectorAll", "closest", "matches"]);

type Literal = ts.StringLiteral | ts.NoSubstitutionTemplateLiteral | ts.TemplateLiteralToken;

function isLiteral(node: ts.Node): node is Literal {
  return (
    ts.isStringLiteral(node) ||
    ts.isNoSubstitutionTemplateLiteral(node) ||
    ts.isTemplateHead(node) ||
    ts.isTemplateMiddle(node) ||
    ts.isTemplateTail(node)
  );
}

/// The literal parts that end up in the value of `expr`: the literal itself,
/// the fixed parts of a template, and the parts of both branches of `?:` and
/// `+`, and of the right side of `??`, `||` and `&&`.
function literalParts(expr: ts.Expression, out: Literal[] = []): Literal[] {
  if (
    ts.isParenthesizedExpression(expr) ||
    ts.isAsExpression(expr) ||
    ts.isSatisfiesExpression(expr) ||
    ts.isNonNullExpression(expr) ||
    ts.isTypeAssertionExpression(expr)
  ) {
    return literalParts(expr.expression, out);
  }
  if (isLiteral(expr)) {
    out.push(expr);
  } else if (ts.isTemplateExpression(expr)) {
    out.push(expr.head, ...expr.templateSpans.map((span) => span.literal));
  } else if (ts.isConditionalExpression(expr)) {
    literalParts(expr.whenTrue, out);
    literalParts(expr.whenFalse, out);
  } else if (ts.isBinaryExpression(expr)) {
    const op = expr.operatorToken.kind;
    if (op === ts.SyntaxKind.PlusToken) literalParts(expr.left, out);
    if (
      op === ts.SyntaxKind.PlusToken ||
      op === ts.SyntaxKind.QuestionQuestionToken ||
      op === ts.SyntaxKind.BarBarToken ||
      op === ts.SyntaxKind.AmpersandAmpersandToken
    ) {
      literalParts(expr.right, out);
    }
  }
  return out;
}

function calleeName(call: ts.CallExpression | ts.NewExpression): string | undefined {
  const callee = call.expression;
  if (ts.isIdentifier(callee)) return callee.text;
  return ts.isPropertyAccessExpression(callee) ? callee.name.text : undefined;
}

function isPlainCall(call: ts.CallExpression): boolean {
  return ts.isIdentifier(call.expression);
}

/// The arguments of a call that become text the person sees.
function textArguments(call: ts.CallExpression): ts.Expression[] {
  const name = calleeName(call);
  if (name === undefined) return [];
  const args = [...call.arguments];
  if (NODE_CALLS.has(name)) return args;
  if (name === "setAttribute" && !isPlainCall(call)) {
    const [attribute, value] = args;
    const known = attribute && isLiteral(attribute) && TEXT_ATTRIBUTES.has(attribute.text);
    return known && value ? [value] : [];
  }
  const helper = isPlainCall(call) ? HELPER_ARGS.get(name) : undefined;
  if (helper === undefined) return [];
  return helper.rest ? args.slice(helper.from) : args.slice(helper.from, helper.from + 1);
}

function isConsoleCall(call: ts.CallExpression): boolean {
  const callee = call.expression;
  return (
    ts.isPropertyAccessExpression(callee) &&
    ts.isIdentifier(callee.expression) &&
    callee.expression.text === "console"
  );
}

function isClassListCall(call: ts.CallExpression): boolean {
  const callee = call.expression;
  if (!ts.isPropertyAccessExpression(callee)) return false;
  if (SELECTOR_CALLS.has(callee.name.text)) return true;
  const owner = callee.expression;
  const ownerName = ts.isPropertyAccessExpression(owner)
    ? owner.name.text
    : ts.isIdentifier(owner)
      ? owner.text
      : undefined;
  return CLASS_LIST_CALLS.has(callee.name.text) && ownerName === "classList";
}

/// True for a child that holds a class list, a selector, a rel value or
/// a log line: sentence-like text there is not shown text. The exemption
/// is by position, never by the look of the text.
function isNotShownText(parent: ts.Node, child: ts.Node): boolean {
  if (ts.isCallExpression(parent)) {
    if (isConsoleCall(parent) || isClassListCall(parent)) return true;
    return calleeName(parent) === "text" && isPlainCall(parent) && parent.arguments[2] === child;
  }
  return (
    ts.isBinaryExpression(parent) &&
    parent.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
    parent.right === child &&
    ts.isPropertyAccessExpression(parent.left) &&
    CLASS_ASSIGNMENTS.has(parent.left.name.text)
  );
}

/// The expressions that a node puts in front of the person (rule 1 of the
/// check).
function shownExpressions(node: ts.Node): ts.Expression[] {
  if (ts.isCallExpression(node)) return textArguments(node);
  if (ts.isNewExpression(node)) {
    const name = ts.isIdentifier(node.expression) ? node.expression.text : "";
    const first = node.arguments?.[0];
    return first && (name === "Option" || name.endsWith("Error")) ? [first] : [];
  }
  if (ts.isBinaryExpression(node)) {
    const { left, operatorToken, right } = node;
    const sink =
      operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isPropertyAccessExpression(left) &&
      TEXT_PROPERTIES.has(left.name.text);
    return sink ? [right] : [];
  }
  if (ts.isPropertyAssignment(node)) {
    const key = ts.isIdentifier(node.name) || ts.isStringLiteral(node.name) ? node.name.text : "";
    return TEXT_KEYS.has(key) ? [node.initializer] : [];
  }
  return [];
}

/// Finds visible text that is written in the source and does not come from
/// the string catalog. Rule 1: a string literal that reaches a place where
/// text is shown (a text property, an attribute, an `append` argument, a
/// Hub helper, an `Error` message, a table of labels). Rule 2: any string
/// literal anywhere that reads like a sentence, except a class list, a
/// selector and a log line. One line per problem, `file:line: "text" ...`.
export function findVisibleText(source: string, fileName = "source.ts"): string[] {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ES2022, true);
  const found = new Map<number, string>();
  const report = (node: Literal): void => {
    const start = node.getStart(file);
    const line = file.getLineAndCharacterOfPosition(start).line + 1;
    found.set(start, `${fileName}:${line}: "${node.text}" is visible text; use t()`);
  };

  const visit = (node: ts.Node, notShown: boolean): void => {
    if (ts.isTypeNode(node)) return;
    for (const expr of shownExpressions(node)) {
      for (const part of literalParts(expr)) if (LETTER.test(part.text)) report(part);
    }
    if (isLiteral(node) && !notShown && SENTENCE.test(node.text)) report(node);
    ts.forEachChild(node, (child) => visit(child, notShown || isNotShownText(node, child)));
  };
  visit(file, false);
  return [...found.entries()].sort(([a], [b]) => a - b).map(([, message]) => message);
}
