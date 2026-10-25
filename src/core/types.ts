import { Schema } from "effect"

/**
 * Values that cross module boundaries. Each shape is defined once, as a Schema: the library returns
 * the derived type and the MCP server publishes the same schema, so the two cannot drift. No behavior
 * here.
 */

export const MatchSchema = Schema.Struct({
  matchId: Schema.Number,
  /** Absolute hltv.org URL. */
  url: Schema.String,
  /** ISO day (YYYY-MM-DD) when the results list said so; null when unknown. */
  date: Schema.NullOr(Schema.String),
  event: Schema.NullOr(Schema.String),
  /** [team1, team2] as HLTV displays them. */
  teams: Schema.Tuple([Schema.String, Schema.String]),
  /** [team1, team2] maps won, or null when the list shows no score. */
  scores: Schema.NullOr(Schema.Tuple([Schema.Number, Schema.Number])),
})
export type Match = typeof MatchSchema.Type

export const DemoRefSchema = Schema.Struct({
  matchId: Schema.Number,
  demoId: Schema.Number,
  /** Absolute /download/demo/<id> URL. */
  url: Schema.String,
  matchUrl: Schema.String,
})
export type DemoRef = typeof DemoRefSchema.Type

export const ArchiveSchema = Schema.Struct({
  path: Schema.String,
  bytes: Schema.Number,
  demoUrl: Schema.String,
  /** HLTV's own file name, e.g. pgl-masters-2026-a-vs-b-bo3-<id>.rar */
  suggestedName: Schema.String,
})
export type Archive = typeof ArchiveSchema.Type

/** One line of the download manifest: the demo URL and the file it produced. */
export const ManifestEntrySchema = Schema.Struct({ demoUrl: Schema.String, fileName: Schema.String })

export type SearchFilter = {
  /** HLTV team id, as in hltv.org/team/<id>/... */
  team?: number
  /** Map name, e.g. de_cache or cache (normalized to de_). */
  map?: string
  /** Max matches to return: default 1, at most one results page (MAX_LIMIT). */
  limit?: number
  /** Results to skip, for paging (100 per page on hltv.org). */
  offset?: number
}
