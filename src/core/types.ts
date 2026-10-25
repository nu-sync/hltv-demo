/** Values that cross module boundaries. No behavior here. */

export type Match = {
  matchId: number
  /** Absolute hltv.org URL. */
  url: string
  /** ISO day (YYYY-MM-DD) when the results list said so; null when unknown. */
  date: string | null
  event: string | null
  /** [team1, team2] as HLTV displays them. */
  teams: [string, string]
  /** [team1, team2] maps won, or null when the list shows no score. */
  scores: [number, number] | null
}

export type DemoRef = {
  matchId: number
  demoId: number
  /** Absolute /download/demo/<id> URL. */
  url: string
  matchUrl: string
}

export type Archive = {
  path: string
  bytes: number
  demoUrl: string
  /** HLTV's own file name, e.g. pgl-masters-2026-a-vs-b-bo3-<id>.rar */
  suggestedName: string
}

export type SearchFilter = {
  /** HLTV team id, as in hltv.org/team/<id>/... */
  team?: number
  /** Map name, e.g. de_cache or cache (normalized to de_). */
  map?: string
  /** Max matches to return. */
  limit: number
  /** Results to skip, for paging (100 per page on hltv.org). */
  offset?: number
}
