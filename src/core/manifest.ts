import { Effect } from "effect"
import { appendFile, mkdir, readFile } from "node:fs/promises"
import { dirname, join } from "node:path"
import { ManifestUnreadable } from "./errors.ts"
import type { ManifestEntrySchema } from "./types.ts"

/**
 * `<out>/.hltv-demo.tsv` maps a demo URL to the file it produced, so a repeat fetch does not pull
 * hundreds of megabytes again. A line whose file is gone is ignored, which makes deleting the archive
 * the way to re-fetch it.
 */
export const MANIFEST = ".hltv-demo.tsv"

export type Entry = typeof ManifestEntrySchema.Type

export const manifestPath = (out: string): string => join(out, MANIFEST)

/** Any failure touching `<out>`'s manifest, as the one error this module rejects with. */
const unreadable = (out: string, cause: unknown): ManifestUnreadable =>
  cause instanceof ManifestUnreadable ? cause : new ManifestUnreadable({ path: manifestPath(out), reason: String(cause) })

export const parse = (text: string): Entry[] =>
  text
    .split("\n")
    .filter((line) => line.trim().length > 0 && !line.startsWith("#"))
    .map((line) => {
      const tab = line.indexOf("\t")
      if (tab <= 0) throw new Error(`malformed line: ${line}`)
      return { demoUrl: line.slice(0, tab), fileName: line.slice(tab + 1) }
    })

export const format = (entry: Entry): string => `${entry.demoUrl}\t${entry.fileName}\n`

export const read = async (out: string): Promise<Entry[]> => {
  let text: string
  try {
    text = await readFile(manifestPath(out), "utf8")
  } catch (cause) {
    if ((cause as NodeJS.ErrnoException).code === "ENOENT") return []
    throw unreadable(out, cause)
  }
  try {
    return parse(text)
  } catch (cause) {
    throw unreadable(out, cause)
  }
}

/** Record a download, ignoring an exact repeat. */
export const record = async (out: string, entry: Entry): Promise<void> => {
  const existing = await read(out)
  if (existing.some((e) => e.demoUrl === entry.demoUrl && e.fileName === entry.fileName)) return
  await mkdir(dirname(manifestPath(out)), { recursive: true })
  try {
    await appendFile(manifestPath(out), format(entry), "utf8")
  } catch (cause) {
    throw unreadable(out, cause)
  }
}

/** The file a previous run downloaded for this demo URL, if it is still on disk. */
export const lookup = async (out: string, demoUrl: string): Promise<string | null> => {
  const hit = (await read(out)).find((entry) => entry.demoUrl === demoUrl)
  if (!hit) return null
  const path = join(out, hit.fileName)
  return (await Bun.file(path).exists()) ? path : null
}

/**
 * Run manifest work inside an Effect. The helpers above reject with ManifestUnreadable; this keeps it a
 * typed failure, where a rejected `Effect.promise` would be a defect (an "internal error" to MCP clients).
 */
export const attempt = <A>(out: string, work: () => Promise<A>): Effect.Effect<A, ManifestUnreadable> =>
  Effect.tryPromise({ try: work, catch: (cause) => unreadable(out, cause) })
