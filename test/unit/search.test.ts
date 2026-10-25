import { describe, expect, test } from "bun:test"
import { dedupe, toMatch } from "../../src/hltv/search.ts"
import type { Match } from "../../src/core/types.ts"

const match = (matchId: number, date: string | null, event = "E"): Match => ({
  matchId,
  url: `https://www.hltv.org/matches/${matchId}/x`,
  date,
  event,
  teams: ["A", "B"],
  scores: [2, 0],
})

describe("toMatch", () => {
  test("reads the id from the link, not from the slug", () => {
    const result = toMatch({
      href: "/matches/2399723/vitality-vs-9z-anything",
      teams: ["Vitality", "9z"],
      scores: [2, 0],
      event: "PGL Masters Bucharest 2026",
      heading: "Results for October 24th 2026",
    })
    expect(result.matchId).toBe(2399723)
    expect(result.url).toBe("https://www.hltv.org/matches/2399723/vitality-vs-9z-anything")
    expect(result.date).toBe("2026-10-24")
    expect(result.teams).toEqual(["Vitality", "9z"])
  })
})

describe("dedupe", () => {
  test("keeps the position but takes the dated copy of a repeated match", () => {
    const result = dedupe([match(1, null), match(1, "2026-10-24"), match(2, "2026-10-11")])
    expect(result.map((m) => m.matchId)).toEqual([1, 2])
    expect(result[0]!.date).toBe("2026-10-24")
  })

  test("leaves distinct matches alone", () => {
    expect(dedupe([match(1, "2026-10-24"), match(2, "2026-10-11")])).toHaveLength(2)
  })
})
