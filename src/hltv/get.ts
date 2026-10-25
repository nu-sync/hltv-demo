import { Duration, Effect } from "effect"
import { stat } from "node:fs/promises"
import { homedir } from "node:os"
import { basename, join, resolve as absolutePath } from "node:path"
import type { HltvError } from "../core/errors.ts"
import * as manifest from "../core/manifest.ts"
import type { Archive } from "../core/types.ts"
import { resultsFilter } from "../core/url.ts"
import { classifyOrFail } from "../classify.ts"
import type { SourceShape } from "../source.ts"
import { resolve } from "./resolve.ts"
import { search } from "./search.ts"

/** Where archives go when a run names no folder. */
export const defaultOut = (): string =>
  process.env.HLTV_DEMO_OUT ?? join(homedir(), ".cache", "hltv-demo", "archives")

/** The pause between the matches of a results URL, so a list is not a burst. */
export const DEFAULT_DELAY_MS = 2000

export type Options = {
  /** Destination folder; default `defaultOut()`. */
  out?: string
  /** How many matches a results URL contributes: default 1, at most MAX_LIMIT. */
  limit?: number
  /** Pause between matches in ms; default DEFAULT_DELAY_MS. */
  delayMs?: number
  /** Human progress, and what was skipped. */
  onNote?: (note: string) => void
}

/**
 * Turn any supported URL into archives on disk.
 *
 * A results URL is the only multi-match case. A match with no demo, or one that fails to download, is a
 * note rather than a failed run: the rest continue. Nothing partial is ever returned or recorded.
 */
export const get = (source: SourceShape, url: string, options: Options): Effect.Effect<Archive[], HltvError> =>
  Effect.gen(function* () {
    const target = yield* classifyOrFail(url)
    // One canonical folder, so every path this run reports is the same string.
    const out = absolutePath(options.out ?? defaultOut())
    const note = (text: string) => options.onNote?.(text)

    /** One demo URL to one archive on disk, skipping a file the manifest already points at. */
    const one = (demoUrl: string): Effect.Effect<Archive, HltvError> =>
      Effect.gen(function* () {
        const known = yield* manifest.attempt(out, () => manifest.lookup(out, demoUrl))
        if (known) {
          note(`skip  ${demoUrl} (already downloaded)`)
          const bytes = yield* Effect.promise(() => stat(known).then((s) => s.size))
          return { path: known, bytes, demoUrl, suggestedName: basename(known) }
        }
        note(`get   ${demoUrl}`)
        const saved = yield* source.download(demoUrl, out)
        yield* manifest.attempt(out, () => manifest.record(out, { demoUrl, fileName: saved.suggestedName }))
        const bytes = yield* Effect.promise(() => stat(saved.path).then((s) => s.size))
        note(`done  ${saved.path} (${(bytes / 1e6).toFixed(0)} MB)`)
        return { path: saved.path, bytes, demoUrl, suggestedName: saved.suggestedName }
      })

    /** Note why a match contributed nothing, so a results run carries on with the next one. */
    const skip = (text: string) =>
      Effect.sync(() => {
        note(text)
        return null
      })

    if (target.kind === "demo") return [yield* one(target.url)]

    if (target.kind === "match") {
      const ref = yield* resolve(source, target.url)
      return [yield* one(ref.url)]
    }

    // Keep the URL's own filter: `results?team=9565&map=de_inferno` must not become plain `/results`.
    const matches = yield* search(source, { ...resultsFilter(target.url), limit: options.limit })
    const delay = Duration.millis(Math.max(options.delayMs ?? DEFAULT_DELAY_MS, 0))
    const archives: Archive[] = []
    for (const [index, match] of matches.entries()) {
      if (index > 0) yield* Effect.sleep(delay)
      const ref = yield* resolve(source, match.url).pipe(
        Effect.catchTag("NoDemoFound", () => skip(`nodemo ${match.url}`)),
      )
      if (ref === null) continue
      const archive = yield* one(ref.url).pipe(
        Effect.catchTag("DownloadFailed", (error) => skip(`error ${ref.url}: ${error.message}`)),
      )
      if (archive !== null) archives.push(archive)
    }
    return archives
  })
