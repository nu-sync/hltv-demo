import { Effect } from "effect"
import { fromHeading } from "../core/dates.ts"
import { NoMatchesFound, type HltvError } from "../core/errors.ts"
import type { Match, SearchFilter } from "../core/types.ts"
import { absolute, resultsUrl } from "../core/url.ts"
import type { SourceShape } from "../source.ts"
import type { RawMatch } from "./extract.ts"

/** The most matches one search returns: one hltv.org results page. Page with `offset` for more. */
export const MAX_LIMIT = 100

/** One results row as a Match, keeping every field HLTV may omit optional. */
export const toMatch = (raw: RawMatch): Match => ({
  matchId: Number(/\/matches\/(\d+)/.exec(raw.href)?.[1] ?? NaN),
  url: absolute(raw.href),
  date: fromHeading(raw.heading),
  event: raw.event,
  teams: raw.teams,
  scores: raw.scores,
})

/**
 * hltv.org shows a match twice: once in the "featured" block, again under its date heading. Keep the
 * first position but prefer the copy that carries a date, so a limit does not spend a slot on a duplicate.
 */
export const dedupe = (matches: Match[]): Match[] => {
  const byId = new Map<number, Match>()
  for (const match of matches) {
    const seen = byId.get(match.matchId)
    if (!seen) {
      byId.set(match.matchId, match)
    } else if (seen.date === null && match.date !== null) {
      byId.set(match.matchId, { ...seen, date: match.date, event: seen.event ?? match.event })
    }
  }
  return [...byId.values()]
}

/** The newest matches on a results page: `limit` of them, default 1 and at most MAX_LIMIT. */
export const search = (source: SourceShape, filter: SearchFilter): Effect.Effect<Match[], HltvError> =>
  Effect.gen(function* () {
    const url = resultsUrl(filter)
    const raw = yield* source.results(url)
    const matches = dedupe(raw.map(toMatch).filter((match) => Number.isFinite(match.matchId)))
    if (matches.length === 0) return yield* new NoMatchesFound({ url })
    return matches.slice(0, Math.min(Math.max(filter.limit ?? 1, 1), MAX_LIMIT))
  })
