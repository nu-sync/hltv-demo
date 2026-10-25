import { describe, expect, test } from "bun:test"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import * as manifest from "../../src/core/manifest.ts"
import { ManifestUnreadable } from "../../src/core/errors.ts"

const tempOut = () => mkdtemp(join(tmpdir(), "hltv-demo-manifest-"))

describe("manifest", () => {
  test("format and parse round-trip", () => {
    const entry = { demoUrl: "https://www.hltv.org/download/demo/1", fileName: "a-b-vs-c-bo3-x.rar" }
    expect(manifest.parse(manifest.format(entry))).toEqual([entry])
  })

  test("a missing manifest is empty, not an error", async () => {
    expect(await manifest.read(await tempOut())).toEqual([])
  })

  test("recording is idempotent and lookup resolves only while the file exists", async () => {
    const out = await tempOut()
    const demoUrl = "https://www.hltv.org/download/demo/2"
    const file = join(out, "event-a-vs-b-bo3-y.rar")

    expect(await manifest.lookup(out, demoUrl)).toBeNull()
    await writeFile(file, "rar")
    await manifest.record(out, { demoUrl, fileName: "event-a-vs-b-bo3-y.rar" })
    await manifest.record(out, { demoUrl, fileName: "event-a-vs-b-bo3-y.rar" })

    expect(await manifest.read(out)).toHaveLength(1)
    expect(await manifest.lookup(out, demoUrl)).toBe(file)

    await Bun.file(file).delete()
    expect(await manifest.lookup(out, demoUrl)).toBeNull()
  })

  test("a corrupt line is a typed failure, not a silent skip", async () => {
    const out = await tempOut()
    await writeFile(manifest.manifestPath(out), "no-tab-here\n")
    await expect(manifest.read(out)).rejects.toBeInstanceOf(ManifestUnreadable)
  })
})
