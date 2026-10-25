#!/usr/bin/env bun
/**
 * CLI. Argument parsing and formatting live here; every rule lives in src/core or src/hltv.
 *
 * Exit codes: 0 success, 1 a domain failure, 2 usage error or missing prerequisite.
 */
import { Effect } from "effect"
import { explain, type HltvError } from "./core/errors.ts"
import type { Match } from "./core/types.ts"
import { classifyOrFail } from "./classify.ts"
import { withSource } from "./source.ts"
import { get } from "./hltv/get.ts"
import { resolve } from "./hltv/resolve.ts"
import { MAX_LIMIT, search } from "./hltv/search.ts"

const USAGE = `hltv-demo — find and download HLTV demo archives

usage:
  hltv-demo search [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
  hltv-demo resolve <match-url> [--json]
  hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]
  hltv-demo --help

urls: a match page, a results page, or /download/demo/<id>
env:  CHROME=/path/to/chrome  HLTV_DEMO_PROFILE=/path/to/profile  HLTV_DEMO_OUT=/path`

type Options = {
  team?: number
  map?: string
  limit?: number
  offset?: number
  out?: string
  delayMs?: number
  headless: boolean
  json: boolean
}

/** A command line that parsed: `resolve` and `download` always carry their URL. */
type Flags = Options & ({ command: "search" } | { command: "resolve" | "download"; url: string })

function usage(message: string): never {
  console.error(`hltv-demo: ${message}\n\n${USAGE}`)
  process.exit(2)
}

const parse = (argv: string[]): Flags | "help" => {
  const [command = "--help", ...rest] = argv
  if (command === "--help" || command === "-h" || command === "help") return "help"
  if (command !== "search" && command !== "resolve" && command !== "download") usage(`unknown command: ${command}`)

  const options: Options = { headless: false, json: false }
  let url: string | undefined
  const at = (flag: string, index: number): string => {
    const next = rest[index + 1]
    if (next === undefined || next.startsWith("--")) usage(`${flag} needs a value`)
    return next
  }
  const whole = (flag: string, raw: string, min = 0): number => {
    const value = Number(raw)
    if (!Number.isInteger(value) || value < min) usage(`${flag} must be a whole number${min > 0 ? ` ≥ ${min}` : ""}`)
    return value
  }

  for (let i = 0; i < rest.length; i++) {
    const arg = rest[i]!
    if (arg === "--json") {
      options.json = true
    } else if (arg === "--headless") {
      options.headless = true
    } else if (arg === "--team") {
      options.team = whole(arg, at(arg, i++))
    } else if (arg === "--map") {
      options.map = at(arg, i++)
    } else if (arg === "--limit") {
      options.limit = whole(arg, at(arg, i++), 1)
      if (options.limit > MAX_LIMIT) usage(`--limit must be between 1 and ${MAX_LIMIT}; page with --offset instead`)
    } else if (arg === "--offset") {
      options.offset = whole(arg, at(arg, i++))
    } else if (arg === "--out") {
      options.out = at(arg, i++)
    } else if (arg === "--delay") {
      options.delayMs = whole(arg, at(arg, i++))
    } else if (arg.startsWith("-")) {
      usage(`unknown flag: ${arg}`)
    } else if (url === undefined) {
      url = arg
    } else {
      usage(`one URL at a time (got ${url} and ${arg})`)
    }
  }
  if (command === "search") return { ...options, command }
  if (url === undefined) usage(`${command} needs a URL`)
  return { ...options, command, url }
}

const formatMatch = (match: Match): string =>
  [
    String(match.matchId).padEnd(9),
    (match.date ?? "?").padEnd(11),
    match.teams.join(" vs ").padEnd(42),
    (match.scores ? match.scores.join("-") : "-").padEnd(5),
    match.event ?? "",
    match.url,
  ].join(" ")

/** Run one command; what it prints on stdout, one line per result (JSON with --json). */
const run = (flags: Flags): Effect.Effect<string[], HltvError> =>
  Effect.gen(function* () {
    const line = (value: object, human: string) => (flags.json ? JSON.stringify(value) : human)
    if (flags.command === "search") {
      const matches = yield* withSource((source) =>
        search(source, { team: flags.team, map: flags.map, limit: flags.limit, offset: flags.offset }),
      )
      return matches.map((match) => line(match, formatMatch(match)))
    }
    // A URL that is not on hltv.org fails here, before anything launches a browser.
    yield* classifyOrFail(flags.url)
    if (flags.command === "resolve") {
      const ref = yield* withSource((source) => resolve(source, flags.url))
      return [line(ref, ref.url)]
    }
    const archives = yield* withSource((source) =>
      get(source, flags.url, {
        out: flags.out,
        limit: flags.limit,
        delayMs: flags.delayMs,
        onNote: (text) => {
          if (!flags.json) console.error(text)
        },
      }),
    )
    return archives.map((archive) => line(archive, archive.path))
  })

/** Exit 2 for what the user must fix in their environment; 1 for everything else. */
const exitCode = (error: HltvError): 1 | 2 =>
  error._tag === "BrowserMissing" || error._tag === "ProfileBusy" ? 2 : 1

const main = async (): Promise<number> => {
  const flags = parse(process.argv.slice(2))
  if (flags === "help") {
    console.log(USAGE)
    return 0
  }
  // Not a supported mode: headless Chrome is challenged, and the flag exists to make that legible.
  if (flags.headless) process.env.HLTV_DEMO_HEADLESS = "1"
  const program = run(flags).pipe(
    Effect.flatMap((lines) =>
      Effect.sync(() => {
        // Only a results download comes back empty: every match lacked a demo or failed.
        if (lines.length === 0) {
          console.error("hltv-demo: nothing downloaded")
          return 1
        }
        for (const line of lines) console.log(line)
        return 0
      }),
    ),
    Effect.catch((error) =>
      Effect.sync(() => {
        console.error(explain(error))
        return exitCode(error)
      }),
    ),
  )
  return Effect.runPromise(program).catch((defect: unknown) => {
    console.error(`hltv-demo: ${String(defect)}`)
    return 1
  })
}

process.exit(await main())
