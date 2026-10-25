import { describe, expect, test } from "bun:test"
import { absolute, classify, mapFilter, pageKey, pathOf, resultsUrl } from "../../src/core/url.ts"
import { NotHltvUrl, UnsupportedPage } from "../../src/core/errors.ts"

describe("pathOf", () => {
  test("accepts hltv.org, www.hltv.org, bare paths, and http", () => {
    expect(pathOf("https://www.hltv.org/results?team=1")).toBe("/results?team=1")
    expect(pathOf("https://hltv.org/matches/1/x")).toBe("/matches/1/x")
    expect(pathOf("http://hltv.org/results")).toBe("/results")
    expect(pathOf("/download/demo/9")).toBe("/download/demo/9")
  })

  test("refuses every other host, including look-alikes", () => {
    for (const url of ["https://example.com/results", "https://hltv.org.evil.com/results", "https://not-hltv.org/x"]) {
      expect(() => pathOf(url)).toThrow(NotHltvUrl)
    }
  })
})

describe("pageKey", () => {
  test("identifies by match and demo id, ignoring the slug", () => {
    expect(pageKey("https://www.hltv.org/matches/2399723/vitality-vs-9z")).toBe("/matches/2399723")
    expect(pageKey("/matches/2399723/other-slug")).toBe("/matches/2399723")
    expect(pageKey("https://www.hltv.org/download/demo/113159")).toBe("/download/demo/113159")
  })

  test("keeps path and query for everything else", () => {
    expect(pageKey("https://www.hltv.org/results?team=9565&map=de_cache")).not.toBe(
      pageKey("https://www.hltv.org/results?team=9565&map=de_inferno"),
    )
  })
})

describe("classify", () => {
  test("recognizes the three shapes", () => {
    expect(classify("https://www.hltv.org/matches/2399725/g2-vs-navi")).toMatchObject({ kind: "match", matchId: 2399725 })
    expect(classify("/results?team=9565")).toMatchObject({ kind: "results" })
    expect(classify("/download/demo/113166")).toMatchObject({ kind: "demo", demoId: 113166 })
  })

  test("refuses other hltv.org pages", () => {
    expect(() => classify("https://www.hltv.org/player/7998/s1mple")).toThrow(UnsupportedPage)
    expect(() => classify("https://www.hltv.org/team/9565/vitality")).toThrow(UnsupportedPage)
  })
})

describe("urls we build", () => {
  test("map filter gets the de_ prefix once", () => {
    expect(mapFilter("cache")).toBe("de_cache")
    expect(mapFilter("de_cache")).toBe("de_cache")
    expect(mapFilter("Anubis")).toBe("de_anubis")
    expect(() => mapFilter("cache&x=1")).toThrow()
  })

  test("resultsUrl encodes only what it was given", () => {
    expect(resultsUrl({ team: 9565, map: "cache" })).toBe("https://www.hltv.org/results?team=9565&map=de_cache")
    expect(resultsUrl({})).toBe("https://www.hltv.org/results")
    expect(resultsUrl({ offset: 100 })).toBe("https://www.hltv.org/results?offset=100")
  })

  test("absolute", () => {
    expect(absolute("/download/demo/1")).toBe("https://www.hltv.org/download/demo/1")
    expect(absolute("https://hltv.org/results")).toBe("https://www.hltv.org/results")
  })
})
