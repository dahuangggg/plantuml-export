import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

const script = fs.readFileSync(new URL("../src/lsp-stdio.cjs", import.meta.url), "utf8");

test("bundled LSP responds to initialize", async () => {
  const server = startServer();

  server.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  const message = await server.nextMessage();
  server.stop();

  assert.equal(message.id, 1);
  assert.equal(message.result.serverInfo.name, "plantuml-lsp");
  assert.equal(message.result.capabilities.textDocumentSync, 1);
});

test("bundled LSP publishes structural diagnostics on document open", async () => {
  const server = startServer();
  const uri = "file:///workspace/bad.puml";

  server.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  await server.nextMessage();
  server.send({
    jsonrpc: "2.0",
    method: "textDocument/didOpen",
    params: {
      textDocument: {
        uri,
        languageId: "plantuml",
        version: 1,
        text: "@startuml\nAlice -> Bob : hi\n",
      },
    },
  });

  const notification = await server.nextMessage();
  server.stop();

  assert.equal(notification.id, undefined);
  assert.equal(notification.method, "textDocument/publishDiagnostics");
  assert.equal(notification.params.uri, uri);
  assert.equal(notification.params.diagnostics[0].message, "Missing matching @end directive.");
});

test("bundled LSP honors diagnosticsOnChange setting", async () => {
  const server = startServer();
  const uri = "file:///workspace/change.puml";

  server.send({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      initializationOptions: {
        settings: {
          diagnosticsOnChange: false,
        },
      },
    },
  });
  await server.nextMessage();
  server.send({
    jsonrpc: "2.0",
    method: "textDocument/didChange",
    params: {
      textDocument: { uri, version: 2 },
      contentChanges: [{ text: "@startuml\nAlice -> Bob : hi\n" }],
    },
  });

  assert.equal(await server.nextMessageOrUndefined(150), undefined);

  server.send({
    jsonrpc: "2.0",
    method: "textDocument/didSave",
    params: {
      textDocument: { uri },
    },
  });

  const notification = await server.nextMessage();
  server.stop();

  assert.equal(notification.method, "textDocument/publishDiagnostics");
  assert.equal(notification.params.uri, uri);
});

test("bundled LSP maps PlantUML checksyntax output on save", async () => {
  const fakeBinDir = fs.mkdtempSync(path.join(os.tmpdir(), "plantuml-lsp-bin-"));
  const fakePlantUml = path.join(fakeBinDir, "plantuml");
  fs.writeFileSync(
    fakePlantUml,
    "#!/bin/sh\nprintf 'Error line 2 in file: %s\\nSome diagram description contains errors\\n' \"$4\"\nexit 200\n",
    { mode: 0o755 },
  );
  const server = startServer({
    env: {
      PATH: `${fakeBinDir}${path.delimiter}${process.env.PATH}`,
    },
  });
  const uri = "file:///workspace/syntax.puml";

  server.send({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
  await server.nextMessage();
  server.send({
    jsonrpc: "2.0",
    method: "textDocument/didOpen",
    params: {
      textDocument: {
        uri,
        languageId: "plantuml",
        version: 1,
        text: "@startuml\nbroken\n@enduml\n",
      },
    },
  });
  await server.nextMessage();
  server.send({
    jsonrpc: "2.0",
    method: "textDocument/didSave",
    params: {
      textDocument: { uri },
    },
  });

  const notification = await server.nextMessage();
  server.stop();

  assert.equal(notification.method, "textDocument/publishDiagnostics");
  assert.deepEqual(notification.params.diagnostics, [
    {
      range: {
        start: { line: 1, character: 0 },
        end: { line: 1, character: 1 },
      },
      severity: 1,
      source: "plantuml",
      message: "Some diagram description contains errors",
    },
  ]);
});

function startServer({ env = {} } = {}) {
  const child = spawn(process.execPath, ["-e", script], {
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      ...process.env,
      ...env,
    },
  });
  let stdout = Buffer.alloc(0);
  const waiters = [];

  child.stdout.on("data", (chunk) => {
    stdout = Buffer.concat([stdout, chunk]);
    flush();
  });

  return {
    send(message) {
      const body = JSON.stringify(message);
      child.stdin.write(`Content-Length: ${Buffer.byteLength(body, "utf8")}\r\n\r\n${body}`);
    },
    nextMessage() {
      const parsed = readOneMessage();
      if (parsed) {
        return Promise.resolve(parsed);
      }
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          reject(new Error("Timed out waiting for LSP message"));
        }, 2000);
        waiters.push((message) => {
          clearTimeout(timer);
          resolve(message);
        });
      });
    },
    nextMessageOrUndefined(timeoutMs) {
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve(readOneMessage());
        }, timeoutMs);
      });
    },
    stop() {
      child.kill();
    },
  };

  function flush() {
    while (waiters.length > 0) {
      const parsed = readOneMessage();
      if (!parsed) {
        return;
      }
      waiters.shift()(parsed);
    }
  }

  function readOneMessage() {
    const headerEnd = stdout.indexOf("\r\n\r\n");
    if (headerEnd === -1) {
      return undefined;
    }
    const header = stdout.slice(0, headerEnd).toString("utf8");
    const match = /^Content-Length: (\d+)$/im.exec(header);
    assert.ok(match, `Missing Content-Length in ${header}`);
    const length = Number(match[1]);
    const start = headerEnd + 4;
    const end = start + length;
    if (stdout.length < end) {
      return undefined;
    }
    const body = stdout.slice(start, end).toString("utf8");
    stdout = stdout.slice(end);
    return JSON.parse(body);
  }
}
