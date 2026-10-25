/**
 * A headed Chrome session. This is the only module that knows Playwright.
 *
 * Two field facts are encoded here rather than left to callers:
 *  - the browser must be headed with a persistent profile: HLTV challenges headless clients;
 *  - every operation gets a fresh page, so "is the DOM ready" can never be answered by the previous page.
 */
import { Effect } from "effect"
import { existsSync, statSync } from "node:fs"
import { mkdir, stat } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"
import { chromium, type BrowserContext, type Page } from "playwright-core"
import { BrowserFailed, BrowserMissing, Challenge, DownloadFailed, ProfileBusy } from "./core/errors.ts"
import { acquire } from "./core/lock.ts"
import { pageKey } from "./core/url.ts"

const DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
const CHALLENGE = /just a moment|security verification|attention required/i

export const profileDir = (): string =>
  process.env.HLTV_DEMO_PROFILE ?? join(homedir(), ".cache", "hltv-demo", "chrome")

export const chromePath = (): string => process.env.CHROME ?? DEFAULT_CHROME

/** The lock that keeps two runs from driving the same profile. */
export const lockPath = (): string => `${profileDir()}.lock`

export const isChallengeTitle = (title: string): boolean => CHALLENGE.test(title)

/** Run a browser call that must succeed, turning a throw into a typed failure instead of a defect. */
const attempt = <A>(call: () => Promise<A>): Effect.Effect<A, BrowserFailed> =>
  Effect.tryPromise({ try: call, catch: (cause) => new BrowserFailed({ reason: String(cause) }) })

/** Run a browser call whose failure is an answer rather than an error, settling on `fallback`. */
const settle = <A>(call: () => Promise<A>, fallback: A): Effect.Effect<A> =>
  Effect.promise(() => call().catch(() => fallback))

/** A fresh page for one operation, closed however the operation ends. */
const withPage = <A, E>(
  context: BrowserContext,
  use: (page: Page) => Effect.Effect<A, E>,
): Effect.Effect<A, E | BrowserFailed> =>
  Effect.acquireUseRelease(attempt(() => context.newPage()), use, (page) => settle(() => page.close(), undefined))

const titleOf = (page: Page): Effect.Effect<string> => settle(() => page.title(), "")

export class Session {
  private readonly context: BrowserContext

  private constructor(context: BrowserContext) {
    this.context = context
  }

  /**
   * Launch Chrome on the shared profile and hold the profile lock, both until the scope closes. A
   * profile that cannot be locked, or a Chrome that will not launch, is reported as BrowserMissing.
   */
  static readonly open = (options: { headless?: boolean } = {}) =>
    Effect.gen(function* () {
      const executablePath = chromePath()
      const missing = () => new BrowserMissing({ tried: executablePath })
      if (!existsSync(executablePath)) return yield* missing()
      yield* Effect.acquireRelease(
        Effect.try({ try: () => acquire(lockPath()), catch: missing }).pipe(
          Effect.flatMap((lock) =>
            lock === "busy" ? Effect.fail(new ProfileBusy({ profile: profileDir() })) : Effect.succeed(lock),
          ),
        ),
        (lock) => Effect.sync(() => lock.release()),
      )
      const context = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: () =>
            chromium.launchPersistentContext(profileDir(), {
              executablePath,
              headless: options.headless ?? false,
              acceptDownloads: true,
              args: ["--disable-blink-features=AutomationControlled"],
            }),
          catch: missing,
        }),
        (context) => settle(() => context.close(), undefined),
      )
      return new Session(context)
    })

  /** Open `url` in a fresh page, wait until it arrived and `selector` exists, then read it in-page. */
  read = <A>(url: string, selector: string, extract: () => A): Effect.Effect<A, Challenge | BrowserFailed> =>
    withPage(this.context, (page) =>
      navigate(page, url, selector).pipe(Effect.andThen(attempt(() => page.evaluate(extract)))),
    )

  /**
   * Navigate to a demo URL and save what the browser downloads. The file is returned only after
   * Playwright's download event completes, so a partial archive is impossible.
   */
  downloadTo = (
    url: string,
    out: string,
  ): Effect.Effect<{ path: string; suggestedName: string }, Challenge | DownloadFailed | BrowserFailed> =>
    withPage(this.context, (page) =>
      Effect.gen(function* () {
        // Listen before navigating: a navigation that becomes a download rejects with
        // net::ERR_ABORTED, so the event is the only signal.
        const started = page.waitForEvent("download", { timeout: 10 * 60_000 }).catch(() => null)
        void page.goto(url, { waitUntil: "commit", timeout: 60_000 }).catch(() => undefined)
        const download = yield* Effect.promise(() => started)
        if (download === null) {
          const title = yield* titleOf(page)
          return yield* Effect.fail(
            isChallengeTitle(title)
              ? new Challenge({ url, profile: profileDir() })
              : new DownloadFailed({ url, reason: `no download started (page: ${title || "unknown"})` }),
          )
        }
        const suggestedName = download.suggestedFilename()
        const path = join(out, suggestedName)
        const bytes = yield* Effect.tryPromise({
          try: async () => {
            await mkdir(out, { recursive: true })
            await download.saveAs(path)
            return (await stat(path)).size
          },
          catch: (cause) =>
            new DownloadFailed({ url, reason: `could not save: ${String(cause)}`, bytes: bytesOnDisk(path) }),
        })
        if (bytes === 0) return yield* new DownloadFailed({ url, reason: "empty file", bytes: 0 })
        return { path, suggestedName }
      }),
    )
}

/** Navigate and require both arrival (same page identity) and the selector. */
const navigate = (page: Page, url: string, selector: string): Effect.Effect<void, Challenge> =>
  Effect.gen(function* () {
    const wanted = pageKey(url)
    yield* settle(() => page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 }), null)
    const arrived = yield* settle(
      () => page.waitForURL((candidate) => pageKey(candidate.href) === wanted, { timeout: 60_000 }).then(() => true),
      false,
    )
    const title = yield* titleOf(page)
    if (!arrived || isChallengeTitle(title)) return yield* new Challenge({ url, profile: profileDir() })
    yield* settle(() => page.waitForSelector(selector, { timeout: 60_000 }), null)
  })

const bytesOnDisk = (path: string): number | undefined => {
  try {
    return statSync(path).size
  } catch {
    return undefined
  }
}
