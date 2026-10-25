import { Effect } from "effect"
import type { HltvError } from "./core/errors.ts"
import { Session } from "./browser.ts"
import { readMatches, readMatchPage, type RawMatch, type RawMatchPage } from "./hltv/extract.ts"

/**
 * The browser boundary, and the only thing the operations need. Everything above it is deterministic
 * and testable without a browser: `search`, `resolve`, and `get` take a Source, never a Session.
 */
export type SourceShape = {
  /** The match rows of a results page. */
  readonly results: (url: string) => Effect.Effect<RawMatch[], HltvError>
  /** The match page: demo link, event, date. */
  readonly match: (url: string) => Effect.Effect<RawMatchPage, HltvError>
  /** Download a demo URL into `out`; resolves only when the browser reports completion. */
  readonly download: (
    demoUrl: string,
    out: string,
  ) => Effect.Effect<{ path: string; suggestedName: string }, HltvError>
}

/** Runs one unit of work with a browser, and closes it after. */
export type WithSource = <A, E>(
  use: (source: SourceShape) => Effect.Effect<A, E>,
) => Effect.Effect<A, E | HltvError>

/** Drive a real browser. */
export const fromSession = (session: Session): SourceShape => ({
  results: (url) => session.read(url, ".result-con", readMatches),
  match: (url) => session.read(url, ".timeAndEvent, .streams", readMatchPage),
  download: (demoUrl, out) => session.downloadTo(demoUrl, out),
})

export const headless = (): boolean => process.env.HLTV_DEMO_HEADLESS === "1"

/**
 * One browser per unit of work. A CLI command is one unit; an MCP tool call is one unit. That keeps
 * Chrome from idling for hours behind an MCP server that is only listing tools, and the profile lock
 * keeps two units from sharing a browser anyway.
 */
export const withSource: WithSource = (use) =>
  Effect.scoped(
    Effect.gen(function* () {
      const session = yield* Session.open({ headless: headless() })
      return yield* use(fromSession(session))
    }),
  )
