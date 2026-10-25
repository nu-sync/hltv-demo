/** A Source that answers from memory and writes archive stubs to a temp folder. */
import { Effect } from "effect"
import { writeFile } from "node:fs/promises"
import { mkdtemp } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import type { SourceShape, WithSource } from "../../src/source.ts"
import type { RawMatch, RawMatchPage } from "../../src/hltv/extract.ts"

export type Recorder = { results: string[]; match: string[]; download: string[] }

export const fakeSource = (options: {
  rows?: RawMatch[]
  page?: RawMatchPage | null
  downloadFails?: string[]
} = {}) => {
  const calls: Recorder = { results: [], match: [], download: [] }
  let outDir = ""
  const source: SourceShape = {
    results: (url) =>
      Effect.sync(() => {
        calls.results.push(url)
        return options.rows ?? []
      }),
    match: (url) =>
      Effect.sync(() => {
        calls.match.push(url)
        return (
          options.page ?? { title: "match", demoHref: null, date: null, event: null }
        )
      }),
    download: (demoUrl, out) =>
      Effect.gen(function* () {
        calls.download.push(demoUrl)
        if (options.downloadFails?.includes(demoUrl)) {
          return yield* Effect.fail(new Error("boom") as never)
        }
        outDir = out
        const suggestedName = `${demoUrl.split("/").pop()}-event-a-vs-b-bo3-x.rar`
        const path = join(out, suggestedName)
        yield* Effect.promise(async () => {
          await (await import("node:fs/promises")).mkdir(out, { recursive: true })
          await writeFile(path, "rar")
        })
        return { path, suggestedName }
      }),
  }
  const withSource: WithSource = (use) => use(source)
  return { source, calls, withSource, dir: () => outDir }
}

export const tempDir = () => mkdtemp(join(tmpdir(), "hltv-demo-test-"))

/** A results row that looks like hltv.org's, with a date heading. */
export const row = (matchId: number, date = "Results for October 24th 2026", teams: [string, string] = ["Vitality", "9z"]): RawMatch => ({
  href: `/matches/${matchId}/${teams[0].toLowerCase()}-vs-${teams[1].toLowerCase()}`,
  teams,
  scores: [2, 0],
  event: "PGL Masters Bucharest 2026",
  heading: date,
})
