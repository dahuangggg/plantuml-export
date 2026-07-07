const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

let buffer = Buffer.alloc(0);
const documents = new Map();
let settings = {
  diagnosticsOnChange: true,
};

process.stdin.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  readMessages();
});

function readMessages() {
  while (true) {
    const headerEnd = buffer.indexOf("\r\n\r\n");
    if (headerEnd === -1) {
      return;
    }

    const header = buffer.slice(0, headerEnd).toString("utf8");
    const lengthMatch = /^Content-Length: (\d+)$/im.exec(header);
    if (!lengthMatch) {
      buffer = buffer.slice(headerEnd + 4);
      continue;
    }

    const length = Number(lengthMatch[1]);
    const messageStart = headerEnd + 4;
    const messageEnd = messageStart + length;
    if (buffer.length < messageEnd) {
      return;
    }

    const payload = buffer.slice(messageStart, messageEnd).toString("utf8");
    buffer = buffer.slice(messageEnd);
    handleMessage(JSON.parse(payload));
  }
}

function handleMessage(message) {
  switch (message.method) {
    case "initialize":
      settings = {
        ...settings,
        ...(message.params?.initializationOptions?.settings ?? {}),
      };
      respond(message.id, {
        capabilities: {
          textDocumentSync: 1,
        },
        serverInfo: {
          name: "plantuml-lsp",
          version: "0.0.1",
        },
      });
      break;
    case "initialized":
      break;
    case "textDocument/didOpen":
      updateDocument(
        message.params.textDocument.uri,
        message.params.textDocument.text,
      );
      break;
    case "textDocument/didChange":
      updateDocument(
        message.params.textDocument.uri,
        message.params.contentChanges.at(-1)?.text ?? "",
        settings.diagnosticsOnChange !== false,
      );
      break;
    case "textDocument/didSave":
      publishDiagnostics(message.params.textDocument.uri, { runPlantUml: true });
      break;
    case "shutdown":
      respond(message.id, null);
      break;
    case "exit":
      process.exit(0);
      break;
    default:
      if (Object.prototype.hasOwnProperty.call(message, "id")) {
        respondError(message.id, -32601, `Unknown method: ${message.method}`);
      }
  }
}

function updateDocument(uri, text, publish = true) {
  documents.set(uri, text);
  if (publish) {
    publishDiagnostics(uri);
  }
}

function publishDiagnostics(uri, { runPlantUml = false } = {}) {
  const text = documents.get(uri) ?? "";
  const diagnostics = structuralDiagnostics(text);
  if (diagnostics.length === 0 && runPlantUml) {
    diagnostics.push(...plantUmlSyntaxDiagnostics(text));
  }
  send("textDocument/publishDiagnostics", {
    uri,
    diagnostics,
  });
}

function plantUmlSyntaxDiagnostics(text) {
  const file = path.join(
    os.tmpdir(),
    `zed-plantuml-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}.puml`,
  );

  try {
    fs.writeFileSync(file, text, "utf8");
    const command = syntaxCommand(file);
    const result = childProcess.spawnSync(command.command, command.args, {
      encoding: "utf8",
    });

    if (result.error || result.status === 0) {
      return [];
    }

    return parsePlantUmlOutputDiagnostics(
      `${result.stdout ?? ""}${result.stderr ?? ""}`,
    );
  } finally {
    try {
      fs.unlinkSync(file);
    } catch {
      // best effort temp cleanup
    }
  }
}

function syntaxCommand(file) {
  const jar =
    settings.plantumlJar ||
    process.env.PLANTUML_ZED_PLANTUML_JAR ||
    process.env.PLANTUML_JAR;
  if (jar && fs.existsSync(jar)) {
    return {
      command: settings.javaBinary || process.env.PLANTUML_ZED_JAVA || "java",
      args: ["-jar", jar, "-failfast", "-checksyntax", file],
    };
  }

  return {
    command:
      settings.plantumlBinary ||
      process.env.PLANTUML_ZED_PLANTUML_BIN ||
      "plantuml",
    args: ["-failfast", "-checksyntax", file],
  };
}

function parsePlantUmlOutputDiagnostics(output) {
  const lineMatch = /Error line (\d+) in file:/i.exec(output);
  if (!lineMatch) {
    return [];
  }

  const line = Math.max(Number(lineMatch[1]) - 1, 0);
  const message =
    output
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line && !/^Error line \d+ in file:/i.test(line)) ||
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

function structuralDiagnostics(text) {
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

function respond(id, result) {
  write({ jsonrpc: "2.0", id, result });
}

function respondError(id, code, message) {
  write({ jsonrpc: "2.0", id, error: { code, message } });
}

function send(method, params) {
  write({ jsonrpc: "2.0", method, params });
}

function write(message) {
  const json = JSON.stringify(message);
  process.stdout.write(`Content-Length: ${Buffer.byteLength(json, "utf8")}\r\n\r\n${json}`);
}
