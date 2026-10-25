import { Effect } from "effect"
import type { NotHltvUrl, UnsupportedPage } from "./core/errors.ts"
import { classify, type Target } from "./core/url.ts"

/**
 * `classify` throws so it stays pure and readable; inside an Effect that throw would become a *defect*,
 * which an MCP client reports as an internal error and the CLI cannot explain. This turns it into a
 * typed failure at the boundary where URLs enter the program. `classify` throws only these two.
 */
export const classifyOrFail = (url: string): Effect.Effect<Target, NotHltvUrl | UnsupportedPage> =>
  Effect.try({ try: () => classify(url), catch: (cause) => cause as NotHltvUrl | UnsupportedPage })
