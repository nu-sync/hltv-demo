import { describe, expect, test } from "bun:test"
import { fromHeading } from "../../src/core/dates.ts"

describe("fromHeading", () => {
  test("parses HLTV results headings", () => {
    expect(fromHeading("Results for October 24th 2026")).toBe("2026-10-24")
    expect(fromHeading("Results for January 1st 2027")).toBe("2027-01-01")
    expect(fromHeading("Results for March 22nd 2026")).toBe("2026-03-22")
  })

  test("returns null rather than guessing", () => {
    expect(fromHeading(null)).toBeNull()
    expect(fromHeading("")).toBeNull()
    expect(fromHeading("Featured results")).toBeNull()
    expect(fromHeading("Results for Octember 24th 2026")).toBeNull()
  })
})
