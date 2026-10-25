import { Duration, Effect } from "effect"
import { resolve as absolutePath } from "node:path"
import { stat } from "node:fs/promises"
import type { Archive } from "../core/types.ts"
import type { HltvError } from "../core/errors.ts"
import { DownloadFailed, NoDemoFound } from "../core/errors.ts"
import { resultsFilter } from "../core/url.ts"
import { classifyOrFail } from "../classify.ts"
import * as manifest from "../core/manifest.ts"
import type { SourceShape } from "../source.ts"
import { resolve } from "./resolve.ts"
import { search } from "./search.ts"

export type Options = {
  out: string
  /** How many matches a results URL contributes. */
  limit: number
  /** Pause between matches, so a list is not a burst. */
  delayMs: number
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
    const out = absolutePath(options.out)
    const archives: Archive[] = []
    const note = (text: string) => options.onNote?.(text)

    /** One demo URL to one archive on disk, skipping a file the manifest already points at. */
    const one = (demoUrl: string): Effect.Effect<Archive, HltvError> =>
      Effect.gen(function* () {
        const known = yield* Effect.promise(() => manifest.lookup(out, demoUrl))
        if (known) {
          note(`skip  ${demoUrl} (already downloaded)`)
          const bytes = yield* Effect.promise(() => stat(known).then((s) => s.size))
          return { path: known, bytes, demoUrl, suggestedName: known.split("/").pop() ?? "" }
        }
        note(`get   ${demoUrl}`)
        const saved = yield* source.download(demoUrl, out)
        yield* Effect.promise(() => manifest.record(out, { demoUrl, fileName: saved.suggestedName }))
        const bytes = yield* Effect.promise(() => stat(saved.path).then((s) => s.size))
        note(`done  ${saved.path} (${(bytes / 1e6).toFixed(0)} MB)`)
        return { path: saved.path, bytes, demoUrl, suggestedName: saved.suggestedName }
      })

    if (target.kind === "demo") {
      archives.push(yield* one(target.url))
      return archives
    }

    if (target.kind === "match") {
      const ref = yield* resolve(source, target.url)
      archives.push(yield* one(ref.url))
      return archives
    }

    // Keep the URL's own filter: `results?team=9565&map=de_inferno` must not become plain `/results`.
    const filter = resultsFilter(target.url)
    const matches = yield* search(source, { ...filter, limit: options.limit })
    for (const [index, match] of matches.entries()) {
      if (index > 0) yield* Effect.sleep(Duration.millis(options.delayMs))
      const ref = yield* resolve(source, match.url).pipe(
        Effect.catchTag("NoDemoFound", (error) => Effect.succeed(error)),
      )
      if (ref instanceof NoDemoFound) {
        note(`nodemo ${match.url}`)
        continue
      }
      const archive = yield* one(ref.url).pipe(
        Effect.catchTag("DownloadFailed", (error) => Effect.succeed(error)),
      )
      if (archive instanceof DownloadFailed) {
        note(`error ${ref.url}: ${archive.message}`)
        continue
      }
      archives.push(archive)
    }
    return archives
  })
