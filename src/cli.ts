#!/usr/bin/env bun
/**
 * CLI. Argument parsing and formatting live here; every rule lives in src/core or src/hltv.
 *
 * Exit codes: 0 success, 1 a domain failure, 2 usage error or missing prerequisite.
 */
import { Effect } from "effect"
import { BrowserMissing, ProfileBusy, explain, type HltvError } from "./core/errors.ts"
import { classifyOrFail } from "./classify.ts"
import { withSource } from "./source.ts"
import { search } from "./hltv/search.ts"
import { resolve as resolveDemo } from "./hltv/resolve.ts"
import { get } from "./hltv/get.ts"

const USAGE = `hltv-demo — find and download HLTV demo archives

usage:
  hltv-demo search [--team ID] [--map NAME] [--limit N] [--offset N] [--json]
  hltv-demo resolve <match-url> [--json]
  hltv-demo download <url> [--out DIR] [--limit N] [--delay MS] [--json]
  hltv-demo --help

urls: a match page, a results page, or /download/demo/<id>
env:  CHROME=/path/to/chrome  HLTV_DEMO_PROFILE=/path/to/profile  HLTV_DEMO_OUT=/path`

type Command = "search" | "resolve" | "download"

type Flags = {
  command: Command | "help"
  url?: string
  team?: number
  map?: string
  limit: number
  offset?: number
  out: string
  delayMs: number
  headless: boolean
  json: boolean
}

function usage(message: string): never {
  console.error(`hltv-demo: ${message}\n\n${USAGE}`)
  process.exit(2)
}

const defaultOut = (): string =>
  process.env.HLTV_DEMO_OUT ?? `${process.env.HOME}/.cache/hltv-demo/archives`

const parse = (argv: string[]): Flags => {
  const [first = "--help", ...rest] = argv
  if (first === "--help" || first === "-h" || first === "help") {
    return { command: "help", limit: 1, out: defaultOut(), delayMs: 2000, headless: false, json: false }
  }
  if (!["search", "resolve", "download"].includes(first)) usage(`unknown command: ${first}`)

  const flags: Flags = {
    command: first as Command,
    limit: 1,
    out: defaultOut(),
    delayMs: 2000,
    headless: false,
    json: false,
  }
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
      flags.json = true
    } else if (arg === "--headless") {
      flags.headless = true
    } else if (arg === "--team") {
      flags.team = whole(arg, at(arg, i++))
    } else if (arg === "--map") {
      flags.map = at(arg, i++)
    } else if (arg === "--limit") {
      flags.limit = whole(arg, at(arg, i++), 1)
      if (flags.limit > 100) usage("--limit must be between 1 and 100; page with --offset instead")
    } else if (arg === "--offset") {
      flags.offset = whole(arg, at(arg, i++))
    } else if (arg === "--out") {
      flags.out = at(arg, i++)
    } else if (arg === "--delay") {
      flags.delayMs = whole(arg, at(arg, i++))
    } else if (arg.startsWith("-")) {
      usage(`unknown flag: ${arg}`)
    } else if (flags.url === undefined) {
      flags.url = arg
    } else {
      usage(`one URL at a time (got ${flags.url} and ${arg})`)
    }
  }
  if (flags.command !== "search" && flags.url === undefined) usage(`${flags.command} needs a URL`)
  return flags
}

const printMatch = (json: boolean) => (match: {
  matchId: number
  date: string | null
  teams: [string, string]
  scores: [number, number] | null
  event: string | null
  url: string
}): void => {
  if (json) return console.log(JSON.stringify(match))
  const score = match.scores ? match.scores.join("-") : "-"
  console.log(
    [String(match.matchId).padEnd(9), (match.date ?? "?").padEnd(11), match.teams.join(" vs ").padEnd(42), score.padEnd(5), match.event ?? "", match.url].join(" "),
  )
}

const main = async (): Promise<number> => {
  const flags = parse(process.argv.slice(2))
  if (flags.command === "help") {
    console.log(USAGE)
    return 0
  }
  const note = (text: string) => {
    if (!flags.json) console.error(text)
  }

  try {
    // A URL that is not on hltv.org fails here, before anything launches a browser.
    if (flags.command !== "search") {
      try {
        classifyOrFail(flags.url!).pipe(Effect.runSync)
      } catch (error) {
        console.error(explain(error as HltvError))
        return 1
      }
    }

    if (flags.command === "search") {
      const matches = await Effect.runPromise(
        withSource((source) => search(source, { team: flags.team, map: flags.map, limit: flags.limit, offset: flags.offset })),
      )
      for (const match of matches) printMatch(flags.json)(match)
      return 0
    }

    if (flags.command === "resolve") {
      const ref = await Effect.runPromise(withSource((source) => resolveDemo(source, flags.url!)))
      console.log(flags.json ? JSON.stringify(ref) : ref.url)
      return 0
    }

    const archives = await Effect.runPromise(
      withSource((source) =>
        get(source, flags.url!, { out: flags.out, limit: flags.limit, delayMs: flags.delayMs, onNote: note }),
      ),
    )
    if (archives.length === 0) {
      console.error("hltv-demo: nothing downloaded")
      return 1
    }
    for (const archive of archives) console.log(flags.json ? JSON.stringify(archive) : archive.path)
    return 0
  } catch (error) {
    if (error instanceof BrowserMissing || error instanceof ProfileBusy) {
      console.error(explain(error))
      return 2
    }
    if (error instanceof Error && "remediation" in error) {
      console.error(explain(error as HltvError))
      return 1
    }
    console.error(`hltv-demo: ${String(error)}`)
    return 1
  }
}

process.exit(await main())
