import { describe, expect, test } from "bun:test"
import { mkdtemp, readFile, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { acquire } from "../../src/core/lock.ts"
import { resultsFilter } from "../../src/core/url.ts"

describe("acquire", () => {
  test("one holder, then busy", async () => {
    const path = join(await mkdtemp(join(tmpdir(), "hltv-demo-lock-")), "profile.lock")
    const lock = acquire(path)
    expect(lock).not.toBe("busy")
    if (lock === "busy") return
    expect(await readFile(path, "utf8")).toBe(String(process.pid))
    expect(acquire(path)).toBe("busy")
    lock.release()
    expect(acquire(path)).not.toBe("busy")
  })

  test("a dead holder is a crash, not a lock", async () => {
    const path = join(await mkdtemp(join(tmpdir(), "hltv-demo-lock-")), "profile.lock")
    await writeFile(path, "999999999")
    const lock = acquire(path)
    expect(lock).not.toBe("busy")
    if (lock !== "busy") lock.release()
  })
})

describe("resultsFilter", () => {
  test("keeps the filter a URL already carries", () => {
    expect(resultsFilter("https://www.hltv.org/results?team=9565&map=de_inferno")).toEqual({
      team: 9565,
      map: "de_inferno",
      offset: undefined,
    })
    expect(resultsFilter("/results?offset=100")).toEqual({ team: undefined, map: undefined, offset: 100 })
    expect(resultsFilter("https://www.hltv.org/results")).toEqual({ team: undefined, map: undefined, offset: undefined })
  })
})
