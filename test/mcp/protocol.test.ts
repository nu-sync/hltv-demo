/**
 * The MCP server over its real transport: a subprocess speaking newline-delimited JSON-RPC, driven by
 * the official MCP client. Nothing here is stubbed except the folder, so this catches protocol
 * regressions (schema, tool names, error mapping) that calling handlers directly cannot.
 */
import { afterAll, beforeAll, describe, expect, test } from "bun:test"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js"

const REPO = join(import.meta.dir, "..", "..")
const TOOLS = ["search_matches", "resolve_demo", "download_demo", "list_downloads"]

let client: Client
let out: string

/** The SDK's result union is broad; take the text parts and leave the rest alone. */
const textOf = (result: unknown): string => {
  const content = (result as { content?: unknown }).content
  if (!Array.isArray(content)) return ""
  return content
    .map((part) => (typeof part === "object" && part !== null && "text" in part ? String((part as { text: unknown }).text) : ""))
    .join("\n")
}

beforeAll(async () => {
  out = await mkdtemp(join(tmpdir(), "hltv-demo-mcp-"))
  await writeFile(join(out, ".hltv-demo.tsv"), "https://www.hltv.org/download/demo/5\tevent-a-vs-b-bo3-x.rar\n")
  client = new Client({ name: "hltv-demo-test", version: "0.0.0" })
  await client.connect(
    new StdioClientTransport({ command: "bun", args: ["run", join(REPO, "src", "mcp.ts")], stderr: "ignore" }),
  )
})

afterAll(async () => {
  await client?.close()
})

describe("mcp protocol", () => {
  test("advertises the four tools with descriptions and input schemas", async () => {
    const { tools } = await client.listTools()
    expect(tools.map((tool) => tool.name)).toEqual(TOOLS)
    for (const tool of tools) {
      expect(tool.description?.length ?? 0).toBeGreaterThan(20)
      expect(tool.inputSchema.type).toBe("object")
    }
    const search = tools.find((tool) => tool.name === "search_matches")!
    expect(Object.keys((search.inputSchema as { properties?: object }).properties ?? {})).toEqual(
      expect.arrayContaining(["team", "map", "limit"]),
    )
  })

  test("list_downloads answers with the manifest and its entries", async () => {
    const result = await client.callTool({ name: "list_downloads", arguments: { out } })
    const body = JSON.parse(textOf(result))
    expect(body.manifest).toBe(join(out, ".hltv-demo.tsv"))
    expect(body.entries).toEqual([{ demoUrl: "https://www.hltv.org/download/demo/5", fileName: "event-a-vs-b-bo3-x.rar" }])
  })

  test("a bad URL comes back as an error result with a remediation, not a crash", async () => {
    const result = (await client.callTool({
      name: "resolve_demo",
      arguments: { matchUrl: "https://example.com/matches/1/x" },
    })) as unknown as { isError?: boolean }
    const text = textOf(result)
    expect(result.isError).toBe(true)
    expect(text).toContain("is not an hltv.org URL")
    expect(text).toContain("/download/demo/")
  })

  test("the server is still healthy after an error", async () => {
    const { tools } = await client.listTools()
    expect(tools).toHaveLength(TOOLS.length)
  })
})
