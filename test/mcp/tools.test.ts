/**
 * The MCP tool handlers, called directly through a fake browser. This is the layer that decides what a
 * client sees: parameter clamps, and every failure as { message, remediation } instead of a defect.
 */
import { Effect } from "effect"
import { describe, expect, test } from "bun:test"
import { join } from "node:path"
import { makeHandlers } from "../../src/mcp.ts"
import { fakeSource, row, tempDir } from "../helpers/fake-source.ts"

const run = <A, E>(effect: Effect.Effect<A, E>) => Effect.runPromise(effect)
const failed = <A, E>(effect: Effect.Effect<A, E>) => Effect.runPromise(Effect.flip(effect))

describe("search_matches", () => {
  test("returns matches and clamps the limit to 1..100", async () => {
    const fake = fakeSource({ rows: [row(1), row(2), row(3)] })
    const tools = makeHandlers(fake.withSource)

    expect((await run(tools.search_matches({ limit: 0 }))).map((m) => m.matchId)).toEqual([1])
    expect(await run(tools.search_matches({ limit: 500 }))).toHaveLength(3)
  })

  test("failures reach the client as message plus remediation", async () => {
    const tools = makeHandlers(fakeSource({ rows: [] }).withSource)
    const failure = await failed(tools.search_matches({ team: 1 }))
    expect(failure.message).toContain("lists no matches")
    expect(failure.remediation).toContain("team id")
  })
})

describe("resolve_demo", () => {
  test("a match with no demo is a tool failure, not a defect", async () => {
    const tools = makeHandlers(fakeSource().withSource)
    const failure = await failed(tools.resolve_demo({ matchUrl: "/matches/1/x" }))
    expect(failure.message).toContain("has no demo download link")
    expect(failure.remediation).toContain("may not publish a demo")
  })

  test("a non-hltv URL is refused with the accepted shapes", async () => {
    const tools = makeHandlers(fakeSource().withSource)
    const failure = await failed(tools.resolve_demo({ matchUrl: "https://example.com/x" }))
    expect(failure.message).toContain("is not an hltv.org URL")
    expect(failure.remediation).toContain("/download/demo/")
  })

  test("the resolved demo reaches the client as a URL", async () => {
    const fake = fakeSource({ page: { title: "t", demoHref: "/download/demo/113159", date: null, event: null } })
    const ref = await run(makeHandlers(fake.withSource).resolve_demo({ matchUrl: "/matches/2399723/vitality-vs-9z" }))
    expect(ref.url).toBe("https://www.hltv.org/download/demo/113159")
  })
})

describe("download_demo", () => {
  test("downloads into the given folder and returns the archives", async () => {
    const out = await tempDir()
    const fake = fakeSource({ page: { title: "t", demoHref: "/download/demo/42", date: null, event: null } })
    const archives = await run(makeHandlers(fake.withSource).download_demo({ url: "/matches/7/x", out }))
    expect(archives).toHaveLength(1)
    expect(archives[0]!.demoUrl).toBe("https://www.hltv.org/download/demo/42")
    expect(archives[0]!.path.startsWith(out)).toBe(true)
  })

  test("downloading nothing is a failure the client can act on", async () => {
    const out = await tempDir()
    const fake = fakeSource({ rows: [], page: { title: "t", demoHref: null, date: null, event: null } })
    const failure = await failed(makeHandlers(fake.withSource).download_demo({ url: "/results", out }))
    expect(failure.message.length).toBeGreaterThan(0)
    expect(failure.remediation.length).toBeGreaterThan(0)
  })
})

describe("list_downloads", () => {
  test("reads the manifest of a folder", async () => {
    const out = await tempDir()
    const fake = fakeSource()
    const tools = makeHandlers(fake.withSource)
    await run(tools.download_demo({ url: "/download/demo/9", out }))

    const listing = await run(tools.list_downloads({ out }))
    expect(listing.manifest).toBe(join(out, ".hltv-demo.tsv"))
    expect(listing.entries).toHaveLength(1)
    expect(listing.entries[0]!.demoUrl).toBe("https://www.hltv.org/download/demo/9")
    expect(listing.entries[0]!.fileName).toContain("9-event-a-vs-b-bo3-x.rar")
  })

  test("a folder with no manifest lists nothing", async () => {
    const listing = await run(makeHandlers(fakeSource().withSource).list_downloads({ out: await tempDir() }))
    expect(listing.entries).toEqual([])
  })
})
