#!/usr/bin/env bun
/**
 * Call one MCP tool on this project's server over stdio, for scripts and manual checks.
 *
 * Usage: bun run scripts/mcp-call.ts <tool> ['{"json":"arguments"}']
 * Prints the tool's text content on stdout. Exit 1 when the tool returns an error result.
 */
import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js"
import { join } from "node:path"

const [tool, rawArgs = "{}"] = process.argv.slice(2)
if (!tool) {
  console.error("usage: mcp-call.ts <tool> ['{json arguments}']")
  process.exit(2)
}

const client = new Client({ name: "hltv-demo-call", version: "0.0.0" })
await client.connect(
  new StdioClientTransport({
    command: "bun",
    args: ["run", join(import.meta.dir, "..", "src", "mcp.ts")],
    stderr: "inherit",
  }),
)

const result = (await client.callTool({ name: tool, arguments: JSON.parse(rawArgs) })) as {
  isError?: boolean
  content?: { type: string; text: string }[]
}
console.log((result.content ?? []).map((part) => part.text).join("\n"))
await client.close()
process.exit(result.isError ? 1 : 0)
