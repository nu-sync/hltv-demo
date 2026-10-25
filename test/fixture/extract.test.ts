/**
 * Selector drift alarm. These run the production extractors against real HTML saved from hltv.org, in a
 * headless Chrome with no network. Refresh the fixtures with `bun run scripts/capture-fixtures.ts`; if a
 * selector stops matching, the failure is here rather than in a user's download.
 */
import { afterAll, beforeAll, describe, expect, test } from "bun:test"
import { existsSync } from "node:fs"
import { readFile } from "node:fs/promises"
import { join } from "node:path"
import { chromium, type Browser } from "playwright-core"
import { chromePath } from "../../src/browser.ts"
import { readMatches, readMatchPage, type RawMatch, type RawMatchPage } from "../../src/hltv/extract.ts"
import { toMatch } from "../../src/hltv/search.ts"
import { fromHeading } from "../../src/core/dates.ts"

const FIXTURES = join(import.meta.dir, "..", "fixtures")
const CHROME = chromePath()
const hasChrome = existsSync(CHROME)

let browser: Browser | undefined

beforeAll(async () => {
  if (hasChrome) browser = await chromium.launch({ executablePath: CHROME, headless: true })
})

afterAll(async () => {
  await browser?.close()
})

/** Run an extractor the way production does: inside a page, over real HTML. */
const read = async <A>(fixture: string, extractor: () => A): Promise<A> => {
  const page = await browser!.newPage()
  try {
    await page.setContent(await readFile(join(FIXTURES, fixture), "utf8"))
    return await page.evaluate(extractor)
  } finally {
    await page.close()
  }
}

describe.skipIf(!hasChrome)("results page", () => {
  test("every row becomes a match with teams, event, and date", async () => {
    const rows = await read("results-cache.html", readMatches)
    expect(rows.length).toBeGreaterThan(0)

    const first: RawMatch = rows[0]!
    expect(first.href).toMatch(/^\/matches\/\d+\//)
    expect(first.teams[0].length).toBeGreaterThan(0)
    expect(first.teams[1].length).toBeGreaterThan(0)
    expect(first.event).toBeTruthy()

    const match = toMatch(first)
    expect(Number.isFinite(match.matchId)).toBe(true)
    expect(match.url.startsWith("https://www.hltv.org/matches/")).toBe(true)
  })

  test("rows under a date heading get an ISO date, and the date matches the heading", async () => {
    const rows = await read("results-cache.html", readMatches)
    // The first block on a results page is "Featured results" and has no date heading, so only the
    // grouped rows can carry one. That is why date is nullable rather than guessed.
    expect(rows[0]!.heading).toBeNull()
    const dated = rows.filter((row) => row.heading !== null)
    expect(dated.length).toBeGreaterThan(0)
    for (const row of dated) {
      const match = toMatch(row)
      expect(match.date).toMatch(/^\d{4}-\d{2}-\d{2}$/)
      expect(fromHeading(row.heading)).toBe(match.date)
    }
  })

  test("scores are pairs of numbers when the list shows them", async () => {
    const rows = await read("results-cache.html", readMatches)
    const scored = rows.filter((row) => row.scores !== null)
    expect(scored.length).toBeGreaterThan(0)
    for (const row of scored) {
      expect(Number.isInteger(row.scores![0])).toBe(true)
      expect(Number.isInteger(row.scores![1])).toBe(true)
    }
  })
})

describe.skipIf(!hasChrome)("match page", () => {
  test("exposes the demo link, the event, and a date", async () => {
    const page = (await read("match.html", readMatchPage)) as RawMatchPage
    expect(page.demoHref).toMatch(/^\/download\/demo\/\d+/)
    expect(page.event).toBeTruthy()
    expect(page.date).toMatch(/^\d{4}-\d{2}-\d{2}$/)
  })

  test("the results extractor finds no rows on a match page", async () => {
    expect(await read("match.html", readMatches)).toEqual([])
  })
})
