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
import { chromium, type BrowserContext, type Download, type Page } from "playwright-core"
import { BrowserFailed, BrowserMissing, Challenge, DownloadFailed, ProfileBusy } from "./core/errors.ts"
import { acquire, type Lock } from "./core/lock.ts"
import { pageKey } from "./core/url.ts"

const DEFAULT_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
const CHALLENGE = /just a moment|security verification|attention required/i

export const profileDir = (): string =>
  process.env.HLTV_DEMO_PROFILE ?? join(homedir(), ".cache", "hltv-demo", "chrome")

export const chromePath = (): string => process.env.CHROME ?? DEFAULT_CHROME

/** The lock that keeps two runs from driving the same profile. */
export const lockPath = (): string => `${profileDir()}.lock`

export const isChallengeTitle = (title: string): boolean => CHALLENGE.test(title)

/** Run a browser call, turning a throw into a typed failure instead of a defect. */
const attempt = <A>(call: () => Promise<A>): Effect.Effect<A, BrowserFailed> =>
  Effect.tryPromise({ try: call, catch: (cause) => new BrowserFailed({ reason: String(cause) }) })

export class Session {
  private readonly context: BrowserContext

  private constructor(context: BrowserContext) {
    this.context = context
  }

  static readonly open = (options: { headless?: boolean } = {}) =>
    Effect.acquireRelease(
      Effect.tryPromise({
        try: async () => {
          const executablePath = chromePath()
          if (!existsSync(executablePath)) throw new BrowserMissing({ tried: executablePath })
          const lock = acquire(lockPath())
          if (lock === "busy") throw new ProfileBusy({ profile: profileDir() })
          const context = await chromium
            .launchPersistentContext(profileDir(), {
              executablePath,
              headless: options.headless ?? false,
              acceptDownloads: true,
              args: ["--disable-blink-features=AutomationControlled"],
            })
            .catch((cause: unknown) => {
              lock.release()
              throw cause
            })
          return { context, lock }
        },
        catch: (cause) =>
          cause instanceof BrowserMissing || cause instanceof ProfileBusy
            ? cause
            : new BrowserMissing({ tried: chromePath() }),
      }),
      ({ context, lock }: { context: BrowserContext; lock: Lock }) =>
        Effect.promise(async () => {
          await context.close().catch(() => undefined)
          lock.release()
        }),
    ).pipe(Effect.map(({ context }) => new Session(context)))

  /** Open `url` in a fresh page, wait until it arrived and `selector` exists, then read it in-page. */
  read = <A>(url: string, selector: string, extract: () => A): Effect.Effect<A, Challenge | BrowserFailed> => {
    const context = this.context
    return Effect.gen(function* () {
      const page = yield* attempt(() => context.newPage())
      yield* navigate(page, url, selector)
      const value = yield* attempt(() => page.evaluate(extract))
      yield* Effect.promise(() => page.close().catch(() => undefined))
      return value
    })
  }

  /**
   * Navigate to a demo URL and save what the browser downloads. The file is returned only after
   * Playwright's download event completes, so a partial archive is impossible.
   */
  downloadTo = (url: string, out: string): Effect.Effect<{ path: string; suggestedName: string }, Challenge | DownloadFailed | BrowserFailed> => {
    const context = this.context
    return Effect.gen(function* () {
      const page = yield* attempt(() => context.newPage())
      const pending: Promise<Download | Error> = page
        .waitForEvent("download", { timeout: 10 * 60_000 })
        .then(
          (download) => download,
          (cause: unknown) => cause as Error,
        )
      // A navigation that becomes a download rejects with net::ERR_ABORTED; the event is the signal.
      void page.goto(url, { waitUntil: "commit", timeout: 60_000 }).catch(() => undefined)
      const outcome = yield* Effect.promise(() => pending)
      if (outcome instanceof Error) {
        const title = yield* Effect.promise(() => page.title().catch(() => ""))
        yield* Effect.promise(() => page.close().catch(() => undefined))
        return yield* Effect.fail(
          isChallengeTitle(title)
            ? new Challenge({ url, profile: profileDir() })
            : new DownloadFailed({ url, reason: `no download started (page: ${title || "unknown"})` }),
        )
      }
      const suggestedName = outcome.suggestedFilename()
      const path = join(out, suggestedName)
      const saved = yield* Effect.tryPromise({
        try: async () => {
          await mkdir(out, { recursive: true })
          await outcome.saveAs(path)
          return (await stat(path)).size
        },
        catch: (cause) => new DownloadFailed({ url, reason: `could not save: ${String(cause)}`, bytes: bytesOnDisk(path) }),
      })
      yield* Effect.promise(() => page.close().catch(() => undefined))
      if (saved === 0) return yield* Effect.fail(new DownloadFailed({ url, reason: "empty file", bytes: 0 }))
      return { path, suggestedName }
    })
  }
}

/** Navigate and require both arrival (same page identity) and the selector. */
const navigate = (page: Page, url: string, selector: string): Effect.Effect<void, Challenge | BrowserFailed> =>
  Effect.gen(function* () {
    const wanted = pageKey(url)
    yield* Effect.promise(() => page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => undefined))
    const arrived = yield* Effect.promise(() =>
      page
        .waitForURL((candidate) => pageKey(candidate.href) === wanted, { timeout: 60_000 })
        .then(() => true)
        .catch(() => false),
    )
    const title = yield* Effect.promise(() => page.title().catch(() => ""))
    if (!arrived || isChallengeTitle(title)) {
      yield* Effect.fail(new Challenge({ url, profile: profileDir() }))
    }
    yield* Effect.promise(() => page.waitForSelector(selector, { timeout: 60_000 }).catch(() => undefined))
  })

const bytesOnDisk = (path: string): number | undefined => {
  try {
    return statSync(path).size
  } catch {
    return undefined
  }
}
