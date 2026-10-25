/**
 * The operations shared by the CLI and the MCP server, tested against a fake Source. No browser, no
 * network: these are the rules that decide what a URL means, what gets skipped, and what a failure is.
 */
import { Effect } from "effect"
import { describe, expect, test } from "bun:test"
import { writeFile } from "node:fs/promises"
import { join } from "node:path"
import { DownloadFailed } from "../../src/core/errors.ts"
import { get } from "../../src/hltv/get.ts"
import { resolve } from "../../src/hltv/resolve.ts"
import { search } from "../../src/hltv/search.ts"
import { fakeSource, row, tempDir } from "../helpers/fake-source.ts"

const run = <A, E>(effect: Effect.Effect<A, E>) => Effect.runPromise(effect)

describe("search", () => {
  test("asks hltv.org for the team and map it was given", async () => {
    const { source, calls } = fakeSource({ rows: [row(1), row(2)] })
    const matches = await run(search(source, { team: 9565, map: "cache", limit: 2 }))
    expect(calls.results).toEqual(["https://www.hltv.org/results?team=9565&map=de_cache"])
    expect(matches.map((m) => m.matchId)).toEqual([1, 2])
  })

  test("respects the limit and drops the featured duplicate", async () => {
    const { source } = fakeSource({ rows: [row(1, "Results for October 24th 2026"), row(1, ""), row(2), row(3)] })
    const matches = await run(search(source, { limit: 2 }))
    expect(matches.map((m) => m.matchId)).toEqual([1, 2])
    expect(matches[0]!.date).toBe("2026-10-24")
  })

  test("an empty results page is NoMatchesFound with the URL", async () => {
    const { source } = fakeSource({ rows: [] })
    const error = await run(Effect.flip(search(source, { team: 1, limit: 1 })))
    expect(error._tag).toBe("NoMatchesFound")
    expect(error.message).toContain("/results?team=1")
  })
})

describe("resolve", () => {
  test("reads the demo id from the match page", async () => {
    const { source, calls } = fakeSource({
      page: { title: "t", demoHref: "/download/demo/113159", date: "2026-10-24", event: "PGL" },
    })
    const ref = await run(resolve(source, "/matches/2399723/vitality-vs-9z"))
    expect(ref).toEqual({
      matchId: 2399723,
      demoId: 113159,
      url: "https://www.hltv.org/download/demo/113159",
      matchUrl: "https://www.hltv.org/matches/2399723/vitality-vs-9z",
    })
    expect(calls.match).toEqual(["https://www.hltv.org/matches/2399723/vitality-vs-9z"])
  })

  test("a match with no demo is NoDemoFound", async () => {
    const { source } = fakeSource({ page: { title: "t", demoHref: null, date: null, event: null } })
    expect((await run(Effect.flip(resolve(source, "/matches/1/x"))))._tag).toBe("NoDemoFound")
  })

  test("a non-match URL never opens a page", async () => {
    const { source, calls } = fakeSource()
    expect((await run(Effect.flip(resolve(source, "/download/demo/1"))))._tag).toBe("NoDemoFound")
    expect(calls.match).toEqual([])
  })
})

describe("get", () => {
  test("a demo URL downloads once and is recorded in the manifest", async () => {
    const out = await tempDir()
    const { source, calls } = fakeSource()
    const archives = await run(get(source, "/download/demo/113159", { out, limit: 1, delayMs: 0 }))
    expect(archives).toHaveLength(1)
    expect(calls.download).toEqual(["https://www.hltv.org/download/demo/113159"])

    const again = await run(get(source, "/download/demo/113159", { out, limit: 1, delayMs: 0 }))
    expect(again[0]!.path).toBe(archives[0]!.path)
    expect(calls.download).toHaveLength(1) // the second run skipped it
  })

  test("a match URL resolves first", async () => {
    const out = await tempDir()
    const { source, calls } = fakeSource({
      page: { title: "t", demoHref: "/download/demo/42", date: null, event: null },
    })
    const archives = await run(get(source, "https://www.hltv.org/matches/7/x", { out, limit: 1, delayMs: 0 }))
    expect(calls.match).toHaveLength(1)
    expect(archives[0]!.demoUrl).toBe("https://www.hltv.org/download/demo/42")
  })

  test("a results URL keeps its filter and continues past a match with no demo", async () => {
    const out = await tempDir()
    const { source, calls } = fakeSource({ rows: [row(1), row(2)] })
    let seen = 0
    const pageFor = (url: string) => {
      seen += 1
      return seen === 1
        ? { title: "t", demoHref: null, date: null, event: null }
        : { title: "t", demoHref: `/download/demo/${url.match(/\d+/)?.[0]}`, date: null, event: null }
    }
    const withPages = { ...source, match: (url: string) => Effect.sync(() => pageFor(url)) }

    const archives = await run(
      get(withPages, "https://www.hltv.org/results?team=9565&map=de_inferno", { out, limit: 2, delayMs: 0 }),
    )
    expect(calls.results).toEqual(["https://www.hltv.org/results?team=9565&map=de_inferno"])
    expect(calls.download).toHaveLength(1)
    expect(archives).toHaveLength(1)
  })

  test("a results URL continues past a download that fails, and says so", async () => {
    const out = await tempDir()
    const { source, calls } = fakeSource({ rows: [row(1), row(2)] })
    const notes: string[] = []
    const flaky = {
      ...source,
      match: (url: string) =>
        Effect.succeed({ title: "t", demoHref: `/download/demo/${url.match(/\d+/)?.[0]}`, date: null, event: null }),
      download: (demoUrl: string, dir: string) =>
        demoUrl.endsWith("/1")
          ? Effect.fail(new DownloadFailed({ url: demoUrl, reason: "stalled" }))
          : source.download(demoUrl, dir),
    }

    const archives = await run(get(flaky, "/results", { out, limit: 2, delayMs: 0, onNote: (note) => notes.push(note) }))
    expect(archives.map((archive) => archive.demoUrl)).toEqual(["https://www.hltv.org/download/demo/2"])
    expect(calls.download).toEqual(["https://www.hltv.org/download/demo/2"])
    expect(notes).toContain(
      "error https://www.hltv.org/download/demo/1: https://www.hltv.org/download/demo/1 did not download: stalled",
    )
  })

  test("a corrupt manifest is a typed ManifestUnreadable, not a defect", async () => {
    const out = await tempDir()
    await writeFile(join(out, ".hltv-demo.tsv"), "no-tab-here\n")
    const { source, calls } = fakeSource()
    const error = await run(Effect.flip(get(source, "/download/demo/1", { out, limit: 1, delayMs: 0 })))
    expect(error._tag).toBe("ManifestUnreadable")
    expect(calls.download).toEqual([])
  })

  test("a foreign host is refused before anything opens", async () => {
    const out = await tempDir()
    const { source, calls } = fakeSource()
    const error = await run(Effect.flip(get(source, "https://example.com/matches/1/x", { out, limit: 1, delayMs: 0 })))
    expect(error._tag).toBe("NotHltvUrl")
    expect(calls.match).toEqual([])
  })
})
