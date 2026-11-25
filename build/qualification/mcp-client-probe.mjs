import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { Client } from '@modelcontextprotocol/client';
import { StdioClientTransport } from '@modelcontextprotocol/client/stdio';

const [binaryArg, workspaceArg, qpdfArg] = process.argv.slice(2);
assert(binaryArg && workspaceArg && qpdfArg, 'usage: node mcp-client-probe.mjs <anpdf> <workspace> <qpdf>');
const binary = resolve(binaryArg);
const workspace = resolve(workspaceArg);
const qpdf = resolve(qpdfArg);
const transport = new StdioClientTransport({
  command: binary,
  args: ['mcp', '--root', workspace, '--qpdf', qpdf, '--qpdf-exact'],
  cwd: workspace,
  stderr: 'pipe',
});
let stderr = '';
transport.stderr?.on('data', (chunk) => { stderr += chunk.toString('utf8'); });
const client = new Client({ name: 'anpdf-official-client-probe', version: '0.0.0' });
let summary;
try {
  await client.connect(transport, { timeout: 10_000 });
  assert.equal(client.getNegotiatedProtocolVersion(), '2025-11-25');
  assert.equal(client.getServerVersion()?.name, 'anpdf');
  assert.equal(client.getServerCapabilities()?.tools?.listChanged, false);
  await client.ping({ timeout: 5_000 });

  const { tools } = await client.listTools(undefined, { timeout: 10_000 });
  const contract = JSON.parse(readFileSync(new URL('../../contracts/v1/openapi.json', import.meta.url), 'utf8'));
  const expected = Object.values(contract.paths).map((path) => path.post.operationId).sort();
  assert.deepEqual(tools.map((tool) => tool.name).sort(), expected);
  const inspect = tools.find((tool) => tool.name === 'inspectPdf');
  assert(inspect, 'inspectPdf missing from tools/list');
  assert.equal(inspect.inputSchema?.type, 'object');
  assert.equal(inspect.outputSchema?.type, 'object');
  assert.equal(inspect.annotations?.readOnlyHint, true);

  const inputBytes = readFileSync(join(workspace, 'one.pdf'));
  const result = await client.callTool(
    { name: 'inspectPdf', arguments: { input: { path: 'one.pdf' } } },
    { timeout: 15_000 },
  );
  assert.equal(result.isError, false, JSON.stringify(result));
  assert.equal(result.content[0]?.type, 'text');
  const envelope = result.structuredContent;
  assert.equal(envelope?.contractVersion, contract.info.version);
  assert.equal(envelope?.operationId, 'inspectPdf');
  assert.equal(envelope?.result?.pageCount, 1);
  assert.equal(envelope?.result?.bytes, inputBytes.length);
  assert.equal(envelope?.result?.sha256, createHash('sha256').update(inputBytes).digest('hex'));
  assert.equal(envelope?.result?.encrypted, false);
  assert.deepEqual(JSON.parse(result.content[0].text), envelope);

  const missing = await client.callTool(
    { name: 'inspectPdf', arguments: { input: { path: 'missing.pdf' } } },
    { timeout: 10_000 },
  );
  assert.equal(missing.isError, true, JSON.stringify(missing));
  assert.equal(missing.structuredContent, undefined);
  const errorEnvelope = JSON.parse(missing.content[0].text);
  assert.equal(errorEnvelope.contractVersion, contract.info.version);
  assert.equal(errorEnvelope.operationId, 'inspectPdf');
  assert.equal(errorEnvelope.error.code, 'NOT_FOUND');

  summary = {
    officialMcpClient: 'passed',
    protocolVersion: client.getNegotiatedProtocolVersion(),
    tools: tools.length,
    structuredOutput: 'validated-by-official-client',
  };
} finally {
  await client.close().catch(() => {});
}
assert.equal(stderr, '', stderr);
console.log(JSON.stringify(summary));
