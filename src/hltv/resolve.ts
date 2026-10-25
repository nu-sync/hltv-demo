import { Effect } from "effect"
import type { DemoRef } from "../core/types.ts"
import type { HltvError } from "../core/errors.ts"
import { NoDemoFound } from "../core/errors.ts"
import { absolute } from "../core/url.ts"
import { classifyOrFail } from "../classify.ts"
import type { SourceShape } from "../source.ts"
import { readMatchPage } from "./extract.ts"

/** The demo archive URL of a match page, or NoDemoFound when the match publishes none. */
export const resolve = (source: SourceShape, matchUrl: string): Effect.Effect<DemoRef, HltvError> =>
  Effect.gen(function* () {
    const target = yield* classifyOrFail(matchUrl)
    if (target.kind !== "match") return yield* Effect.fail(new NoDemoFound({ url: matchUrl }))
    const page = yield* source.match(absolute(matchUrl))
    if (!page.demoHref) return yield* Effect.fail(new NoDemoFound({ url: matchUrl }))
    return {
      matchId: target.matchId,
      demoId: Number(/\/download\/demo\/(\d+)/.exec(page.demoHref)?.[1] ?? NaN),
      url: absolute(page.demoHref),
      matchUrl: absolute(matchUrl),
    }
  })
