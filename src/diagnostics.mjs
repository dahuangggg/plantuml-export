export function parsePlantUmlOutputDiagnostics({ output }) {
  const lineMatch = /Error line (\d+) in file:/i.exec(output);
  if (!lineMatch) {
    return [];
  }

  const line = Math.max(Number(lineMatch[1]) - 1, 0);
  const message =
    output
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line && !/^Error line \d+ in file:/i.test(line)) ??
    "PlantUML syntax error";

  return [
    {
      range: {
        start: { line, character: 0 },
        end: { line, character: 1 },
      },
      severity: 1,
      source: "plantuml",
      message,
    },
  ];
}

export function structuralDiagnostics(text) {
  const diagnostics = [];
  const starts = [...text.matchAll(/@start[a-zA-Z0-9_]*/g)];
  const ends = [...text.matchAll(/@end[a-zA-Z0-9_]*/g)];

  if (starts.length > ends.length) {
    const match = starts.at(-1);
    const position = offsetToPosition(text, match.index ?? 0);
    diagnostics.push({
      range: {
        start: position,
        end: { line: position.line, character: position.character + match[0].length },
      },
      severity: 1,
      source: "plantuml",
      message: "Missing matching @end directive.",
    });
  }

  if (starts.length === 0 && text.trim().length > 0) {
    diagnostics.push({
      range: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: Math.min(firstLineLength(text), 80) },
      },
      severity: 2,
      source: "plantuml",
      message: "PlantUML files should start with an @start directive.",
    });
  }

  if (starts.length > 0 && starts.length === ends.length && isEmptyDiagram(text)) {
    const start = offsetToPosition(text, starts[0].index ?? 0);
    const endMatch = ends.at(-1);
    const end = offsetToPosition(
      text,
      (endMatch.index ?? 0) + endMatch[0].length,
    );
    diagnostics.push({
      range: { start, end },
      severity: 2,
      source: "plantuml",
      message: "PlantUML diagram has no body.",
    });
  }

  return diagnostics;
}

function isEmptyDiagram(text) {
  return text
    .replace(/@start[a-zA-Z0-9_]*/g, "")
    .replace(/@end[a-zA-Z0-9_]*/g, "")
    .trim().length === 0;
}

function offsetToPosition(text, offset) {
  const prefix = text.slice(0, offset);
  const lines = prefix.split("\n");
  return {
    line: lines.length - 1,
    character: lines.at(-1).length,
  };
}

function firstLineLength(text) {
  const newline = text.indexOf("\n");
  return newline === -1 ? text.length : newline;
}
