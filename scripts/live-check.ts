#!/usr/bin/env bun
/**
 * Live end-to-end check: both entry points, against whatever is newest on https://www.hltv.org/results.
 *
 * The target is discovered, not hardcoded: read the newest results page, walk the candidates until one
 * publishes a demo, then drive the CLI and the MCP server against that match and the same output folder.
 * A hardcoded match URL rots the day its demo is removed; a discovered one keeps the check honest.
 *
 * Exit status: 0 every check passed, 1 a check failed, 2 a prerequisite is missing (a coverage gap, not
 * a pass — Cloudflare's challenge, a held profile, no Chrome, or HLTV simply publishing no demo yet).
 *
 * Environment:
 *   LIVE_SCAN          newest matches to scan for a demo (default 10)
 *   LIVE_MAX_AGE_DAYS  how old the discovered target may be (default 30)
 *   LIVE_KEEP=1        keep the downloaded archive (440-890 MB) instead of deleting it
 *   EVIDENCE           evidence folder root (default <repo>/.live-checks)
 *   CHROME, HLTV_DEMO_PROFILE  as the tool itself reads them
 */
import { Client } from "@modelcontextprotocol/sdk/client/index.js"
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js"
import { Duration, Effect } from "effect"
import { existsSync, statSync } from "node:fs"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, join } from "node:path"
import { chromePath, profileDir } from "../src/browser.ts"
import { explain, type HltvError } from "../src/core/errors.ts"
import * as manifest from "../src/core/manifest.ts"
import type { Archive, DemoRef, Match } from "../src/core/types.ts"
import { BASE } from "../src/core/url.ts"
import { resolve } from "../src/hltv/resolve.ts"
import { search } from "../src/hltv/search.ts"
import type { RawMatchPage } from "../src/hltv/extract.ts"
import { withSource } from "../src/source.ts"

const REPO = join(import.meta.dir, "..")
const CLI = join(REPO, "src", "cli.ts")
const MCP = join(REPO, "src", "mcp.ts")
const SCAN = Math.max(1, Number(process.env.LIVE_SCAN ?? 10))
const MAX_AGE_DAYS = Math.max(0, Number(process.env.LIVE_MAX_AGE_DAYS ?? 30))
const KEEP = process.env.LIVE_KEEP === "1"
const STARTED = Date.now()
const RUN = `${new Date().toISOString().slice(0, 19).replace(/[-:T]/g, "")}-${process.pid}`
const EVIDENCE = join(process.env.EVIDENCE ?? join(REPO, ".live-checks"), RUN)
const MIN_ARCHIVE_BYTES = 100_000_000

// ---------------------------------------------------------------- check harness

type Status = "PASS" | "FAIL" | "GAP"
type Check = { name: string; status: Status; detail?: string; ms: number }

const checks: Check[] = []
const lines: string[] = []
const facts: Record<string, unknown> = {}

const emit = (line: string) => {
  console.log(line)
  lines.push(line)
}

/** A check failed and everything after it depends on it. */
class Failure extends Error {}
/** A check could not run: the environment, not the code, is missing something. */
class Prerequisite extends Error {}

const assert = (condition: unknown, message: string): void => {
  if (!condition) throw new Error(message)
}

/** An environment that is not there: not a code failure, so the run ends at exit 2. */
const require = (condition: unknown, message: string): void => {
  if (!condition) throw new Prerequisite(message)
}

/** Cloudflare, a held profile, and a missing Chrome are gaps. Anything else is a real failure. */
const triage = (error: unknown): { gap: boolean; detail: string } => {
  const tag = (error as { _tag?: string })._tag
  if (tag === "Challenge" || tag === "ProfileBusy" || tag === "BrowserMissing") {
    return { gap: true, detail: explain(error as HltvError) }
  }
  return { gap: false, detail: error instanceof Error ? error.message : String(error) }
}

const check = async (name: string, fn: () => Promise<string | void>): Promise<void> => {
  const started = Date.now()
  try {
    const detail = await fn()
    checks.push({ name, status: "PASS", detail: typeof detail === "string" ? detail : undefined, ms: Date.now() - started })
    emit(`PASS  ${name}${typeof detail === "string" ? ` — ${detail}` : ""}`)
  } catch (error) {
    const { gap, detail } =
      error instanceof Prerequisite ? { gap: true, detail: error.message } : triage(error)
    checks.push({ name, status: gap ? "GAP" : "FAIL", detail, ms: Date.now() - started })
    emit(`${gap ? "GAP " : "FAIL"}  ${name}\n      ${detail.replaceAll("\n", "\n      ")}`)
    throw gap ? new Prerequisite(detail) : new Failure(detail)
  }
}

// ---------------------------------------------------------------- subprocess helpers

type Run = { code: number; stdout: string; stderr: string; ms: number }

const spawn = async (cmd: string[], env: Record<string, string> = {}): Promise<Run> => {
  const started = Date.now()
  const proc = Bun.spawn(cmd, {
    cwd: REPO,
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...env } as Record<string, string>,
  })
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()])
  return { code: await proc.exited, stdout, stderr, ms: Date.now() - started }
}

const cli = (args: string[], env?: Record<string, string>): Promise<Run> => spawn(["bun", "run", CLI, ...args], env)

/** Parse a JSON-per-line or single-JSON CLI answer, keeping the raw text in the failure message. */
const json = <A>(text: string, what: string): A => {
  try {
    return JSON.parse(text.trim()) as A
  } catch {
    throw new Error(`${what} did not print JSON: ${text.trim().slice(0, 400) || "(empty)"}`)
  }
}

/** The archives in a folder, by HLTV's own extension. */
const rars = async (dir: string): Promise<string[]> => {
  const found: string[] = []
  for await (const file of new Bun.Glob("*.rar").scan(dir)) found.push(file)
  return found
}

/** The CLI prints JSON one object per line, so a run of archives is read as lines. */
const jsonLines = <A>(text: string, what: string): A[] =>
  text
    .trim()
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .map((line) => json<A>(line, what))

const textOf = (result: unknown): string => {
  const content = (result as { content?: unknown }).content
  if (!Array.isArray(content)) return ""
  return content
    .map((part) => (typeof part === "object" && part !== null && "text" in part ? String((part as { text: unknown }).text) : ""))
    .join("\n")
}

// ---------------------------------------------------------------- discovery

type Trial = { match: Match; page: Pick<RawMatchPage, "demoHref" | "event" | "date"> }

type Discovery = {
  matches: Match[]
  trials: Trial[]
  hit: Match | null
  demo: DemoRef | null
}

/**
 * One browser, one results page, then match pages until one has a demo. Deliberately sequential: the
 * same politeness the tool promises, and a burst of navigations is how a live check earns a challenge.
 */
const discover = (): Promise<Discovery> =>
  Effect.runPromise(
    withSource((source) =>
      Effect.gen(function* () {
        const matches = yield* search(source, { limit: SCAN })
        const trials: Trial[] = []
        let hit: Match | null = null
        for (const [index, match] of matches.entries()) {
          if (index > 0) yield* Effect.sleep(Duration.millis(750))
          const page = yield* source.match(match.url)
          trials.push({ match, page: { demoHref: page.demoHref, event: page.event, date: page.date } })
          if (page.demoHref) {
            hit = match
            break
          }
        }
        const demo = hit === null ? null : yield* resolve(source, hit.url)
        return { matches, trials, hit, demo } satisfies Discovery
      }),
    ),
  )

// ---------------------------------------------------------------- main

const main = async (): Promise<number> => {
  emit(`hltv-demo live check ${RUN}`)
  emit(`target: whatever is newest on ${BASE}/results`)
  emit(`evidence: ${EVIDENCE}`)
  await mkdir(EVIDENCE, { recursive: true })

  emit("")
  emit("== prerequisites")

  await check("bun, Chrome, bsdtar, and node_modules are present", async () => {
    const chrome = chromePath()
    require(existsSync(chrome), `no Chrome at ${chrome} (set CHROME=)`)
    require(Bun.which("bsdtar") !== null, "bsdtar is not on PATH")
    require(existsSync(join(REPO, "node_modules")), "dependencies are not installed (bun install)")
    facts.chrome = chrome
    facts.profile = profileDir()
    return chrome
  })

  await check("the offline suites pass (bun test)", async () => {
    const run = await spawn(["bun", "test"])
    facts.bunTest = { code: run.code, ms: run.ms, tail: run.stderr.slice(-2000) }
    assert(run.code === 0, `bun test exited ${run.code}\n${run.stderr.slice(-2000)}`)
    const summary = /(\d+) pass\s*\n\s*(\d+) fail/.exec(run.stderr)
    return summary ? `${summary[1]} pass, ${summary[2]} fail` : "exit 0"
  })

  emit("")
  emit("== discovery: anything recent on /results")

  let discovery!: Discovery
  await check(`a recent match publishes a demo (scanned up to ${SCAN})`, async () => {
    discovery = await discover()
    facts.discovery = {
      matches: discovery.matches.slice(0, SCAN),
      trials: discovery.trials,
      hit: discovery.hit,
      demo: discovery.demo,
    }
    assert(discovery.matches.length > 0, `${BASE}/results listed no matches`)
    assert(
      discovery.hit !== null,
      discovery.trials.every((trial) => trial.page.event === null && trial.page.date === null)
        ? `no candidate page yielded an event or a date: the match-page extractor is broken, not HLTV\n${JSON.stringify(discovery.trials, null, 2)}`
        : `none of the newest ${discovery.trials.length} matches publishes a demo yet (HLTV publishes them hours after a match); retry later or raise LIVE_SCAN\n${discovery.trials.map((t) => `  ${t.match.url} event=${t.page.event}`).join("\n")}`,
    )
    return `${discovery.hit!.matchId} ${discovery.hit!.teams.join(" vs ")} (${discovery.trials.length} scanned)`
  })

  const target = discovery.hit!
  const demo = discovery.demo!

  await check("the match page extractor still reads an event and a date", async () => {
    const trial = discovery.trials.find((entry) => entry.match.matchId === target.matchId)!
    assert(trial.page.event !== null, `no event on ${target.url}`)
    assert(trial.page.date !== null, `no date on ${target.url}`)
    return `${trial.page.event}, ${trial.page.date}`
  })

  await check(`the target is recent (${MAX_AGE_DAYS} days)`, async () => {
    const trial = discovery.trials.find((entry) => entry.match.matchId === target.matchId)!
    if (trial.page.date === null) return "undated (featured block); recency taken from position"
    const ageDays = (Date.now() - Date.parse(`${trial.page.date}T00:00:00Z`)) / 86_400_000
    const days = Math.round(ageDays)
    assert(ageDays <= MAX_AGE_DAYS, `${target.url} is ${days} day${days === 1 ? "" : "s"} old (LIVE_MAX_AGE_DAYS=${MAX_AGE_DAYS})`)
    return `${trial.page.date}, ${days} day${days === 1 ? "" : "s"} old`
  })

  await check("the library's resolve() agrees with the page extractor", async () => {
    const trial = discovery.trials.find((entry) => entry.match.matchId === target.matchId)!
    assert(demo.matchId === target.matchId, `resolve() reported match ${demo.matchId} for ${target.matchId}`)
    assert(demo.url.endsWith(trial.page.demoHref!), `resolve() gave ${demo.url}, the page gave ${trial.page.demoHref}`)
    return demo.url
  })

  emit("")
  emit("== cli")

  let cliMatches: Match[] = []
  await check("search --limit 5 --json exits 0 with five unique matches", async () => {
    const run = await cli(["search", "--limit", "5", "--json"])
    facts.cliSearch = run
    assert(run.code === 0, `exit ${run.code}: ${run.stderr.slice(-800)}`)
    cliMatches = jsonLines<Match>(run.stdout, "search")
    assert(cliMatches.length === 5, `got ${cliMatches.length} matches`)
    assert(new Set(cliMatches.map((match) => match.matchId)).size === 5, "duplicate match ids")
    return `${cliMatches.length} matches in ${run.ms} ms`
  })

  await check("every match search returns is an hltv.org match URL", async () => {
    for (const match of cliMatches) {
      assert(match.url.startsWith(`${BASE}/matches/`), `${match.url} is not a match URL`)
      assert(Number.isInteger(match.matchId), `${match.url} has no match id`)
      assert(match.teams[0].length > 0 && match.teams[1].length > 0, `${match.url} has no teams`)
    }
    return `${cliMatches[0]!.date ?? "?"} newest`
  })

  await check("resolve <match> prints the demo URL the scan found", async () => {
    const run = await cli(["resolve", target.url])
    facts.cliResolve = run
    assert(run.code === 0, `exit ${run.code}: ${run.stderr.slice(-800)}`)
    const url = run.stdout.trim()
    assert(/^https:\/\/www\.hltv\.org\/download\/demo\/\d+$/.test(url), `printed ${JSON.stringify(url)}`)
    assert(url === demo.url, `printed ${url}, the page extractor gave ${demo.url}`)
    return url
  })

  const work = await mkdtemp(join(tmpdir(), "hltv-demo-live-"))
  const out = join(work, "archives")

  try {
    let archive!: Archive
    await check("download <match> --out exits 0", async () => {
      const run = await cli(["download", target.url, "--out", out, "--json"])
      facts.cliDownload = run
      assert(run.code === 0, `exit ${run.code}: ${run.stderr.slice(-1500)}`)
      const archives = jsonLines<Archive>(run.stdout, "download")
      assert(Array.isArray(archives) && archives.length === 1, `expected one archive, got ${JSON.stringify(archives)}`)
      archive = archives[0]!
      return `${archive.suggestedName} in ${run.ms} ms`
    })

    await check("the archive is on disk, and only one is", async () => {
      assert(existsSync(archive.path), `${archive.path} is not on disk`)
      const listing = await rars(out)
      assert(listing.length === 1, `expected one archive, found ${JSON.stringify(listing)}`)
      facts.archive = { ...archive, bytesOnDisk: statSync(archive.path).size, listing }
      return basename(archive.path)
    })

    await check(`the archive is a real download (> ${MIN_ARCHIVE_BYTES / 1e6} MB)`, async () => {
      const bytes = statSync(archive.path).size
      assert(bytes > MIN_ARCHIVE_BYTES, `${(bytes / 1e6).toFixed(1)} MB is too small to be a demo archive`)
      assert(bytes === archive.bytes, `reported ${archive.bytes} bytes, on disk ${bytes}`)
      return `${(bytes / 1e6).toFixed(0)} MB`
    })

    await check("the file name is HLTV's own, never renamed", async () => {
      const name = basename(archive.path)
      assert(name === archive.suggestedName, `path says ${name}, the browser suggested ${archive.suggestedName}`)
      assert(/-vs-/i.test(name), `${name} does not name the two teams`)
      assert(name.endsWith(".rar"), `${name} is not an archive`)
      assert(!/\(\d+\)\.rar$/.test(name), `${name} looks like a browser duplicate`)
      return name
    })

    await check("the archive really holds .dem files", async () => {
      const run = await spawn(["bsdtar", "-tf", archive.path])
      assert(run.code === 0, `bsdtar exited ${run.code}: ${run.stderr.slice(-400)}`)
      const dems = run.stdout.split("\n").filter((line) => line.endsWith(".dem"))
      assert(dems.length >= 1, "no .dem in the archive: this is not a demo archive")
      facts.demos = dems
      return `${dems.length} .dem`
    })

    await check("the download is recorded in the manifest", async () => {
      const entries = await manifest.read(out)
      assert(entries.length === 1, `manifest has ${entries.length} entries`)
      assert(entries[0]!.demoUrl === demo.url, `manifest says ${entries[0]!.demoUrl}`)
      assert(entries[0]!.fileName === archive.suggestedName, `manifest says ${entries[0]!.fileName}`)
      return `${manifest.MANIFEST}: ${entries[0]!.fileName}`
    })

    await check("a repeat download exits 0 and skips", async () => {
      const run = await cli(["download", target.url, "--out", out])
      facts.cliRepeat = run
      assert(run.code === 0, `exit ${run.code}: ${run.stderr.slice(-800)}`)
      assert(run.stderr.includes("skip "), `the repeat did not skip: ${run.stderr.slice(-400)}`)
      assert(run.stdout.trim() === archive.path, `the repeat reported ${run.stdout.trim()}`)
      return `skip in ${run.ms} ms`
    })

    await check("the repeat downloaded nothing and grew nothing", async () => {
      const listing = await rars(out)
      assert(listing.length === 1, `a second archive appeared: ${JSON.stringify(listing)}`)
      const entries = await manifest.read(out)
      assert(entries.length === 1, `the manifest grew to ${entries.length} entries`)
      return "one archive, one manifest line"
    })

    emit("")
    emit("== mcp over stdio")

    const transport = new StdioClientTransport({
      command: "bun",
      args: ["run", MCP],
      cwd: REPO,
      env: Object.fromEntries(Object.entries(process.env).filter(([, value]) => value !== undefined)) as Record<string, string>,
      stderr: "pipe",
    })
    const logs: string[] = []
    const serverLog = transport.stderr as unknown as { on?: (event: "data", cb: (chunk: unknown) => void) => void } | null
    serverLog?.on?.("data", (chunk) => logs.push(String(chunk)))

    const client = new Client({ name: "hltv-demo-live", version: "0.0.0" })
    try {
      await client.connect(transport)

      await check("tools/list advertises the four tools", async () => {
        const { tools } = await client.listTools()
        const names = tools.map((tool) => tool.name).sort()
        assert(
          JSON.stringify(names) === JSON.stringify(["download_demo", "list_downloads", "resolve_demo", "search_matches"]),
          `got ${JSON.stringify(names)}`,
        )
        return names.join(", ")
      })

      await check("search_matches returns matches the CLI also saw", async () => {
        const result = await client.callTool({ name: "search_matches", arguments: { limit: 5 } })
        facts.mcpSearch = result
        const matches = json<Match[]>(textOf(result), "search_matches")
        assert(Array.isArray(matches) && matches.length > 0, `got ${matches.length} matches`)
        const seen = new Set(cliMatches.map((match) => match.matchId))
        const shared = matches.filter((match) => seen.has(match.matchId))
        // The results page can gain a match between the two calls; one shared id proves the same source.
        assert(shared.length >= 1, `MCP ${matches.map((m) => m.matchId).join(",")} vs CLI ${cliMatches.map((m) => m.matchId).join(",")}`)
        assert(matches.every((match) => match.url.startsWith(`${BASE}/matches/`)), "a match URL is off hltv.org")
        return `${matches.length} matches, ${shared.length} shared with the CLI`
      })

      await check("resolve_demo returns the same demo URL and match id", async () => {
        const result = await client.callTool({ name: "resolve_demo", arguments: { matchUrl: target.url } })
        facts.mcpResolve = result
        const ref = json<DemoRef>(textOf(result), "resolve_demo")
        assert(ref.url === demo.url, `MCP ${ref.url}, the CLI/library ${demo.url}`)
        assert(ref.matchId === target.matchId, `MCP match ${ref.matchId}, expected ${target.matchId}`)
        assert(ref.demoId === demo.demoId, `MCP demo ${ref.demoId}, expected ${demo.demoId}`)
        return ref.url
      })

      await check("download_demo reuses the archive the CLI downloaded", async () => {
        const result = await client.callTool({ name: "download_demo", arguments: { url: target.url, out } })
        facts.mcpDownload = result
        const archives = json<Archive[]>(textOf(result), "download_demo")
        assert(Array.isArray(archives) && archives.length === 1, `expected one archive, got ${JSON.stringify(archives)}`)
        const mcp = archives[0]!
        assert(mcp.path === archive.path, `MCP wrote ${mcp.path}, the CLI wrote ${archive.path}`)
        assert(mcp.demoUrl === demo.url, `MCP demo URL ${mcp.demoUrl}`)
        assert(mcp.bytes === archive.bytes, `MCP ${mcp.bytes} bytes, the CLI ${archive.bytes}`)
        return `${basename(mcp.path)} (shared manifest, ${mcp.bytes} bytes)`
      })

      await check("download_demo added no second archive", async () => {
        const listing = await rars(out)
        assert(listing.length === 1, `expected one archive after both transports, found ${JSON.stringify(listing)}`)
        return listing[0]!
      })

      await check("list_downloads reports the one entry both transports share", async () => {
        const result = await client.callTool({ name: "list_downloads", arguments: { out } })
        facts.mcpList = result
        const body = json<{ manifest: string; entries: manifest.Entry[] }>(textOf(result), "list_downloads")
        assert(body.manifest === manifest.manifestPath(out), `manifest path ${body.manifest}`)
        assert(body.entries.length === 1, `${body.entries.length} entries`)
        assert(body.entries[0]!.demoUrl === demo.url, `entry demo URL ${body.entries[0]!.demoUrl}`)
        return body.manifest
      })

      await check("an off-site URL is an error result with a remediation, not a crash", async () => {
        const result = (await client.callTool({
          name: "resolve_demo",
          arguments: { matchUrl: "https://example.com/matches/1/x" },
        })) as unknown as { isError?: boolean }
        facts.mcpError = result
        const text = textOf(result)
        assert(result.isError === true, "the tool returned success for an off-site URL")
        assert(text.includes("is not an hltv.org URL"), `text was ${JSON.stringify(text.slice(0, 300))}`)
        assert(text.includes("/download/demo/"), "no remediation in the error")
        return text.split("\n")[0]!
      })

      await check("the server is still healthy after an error", async () => {
        const { tools } = await client.listTools()
        assert(tools.length === 4, `${tools.length} tools after the error`)
        return "four tools, still answering"
      })
    } finally {
      await client.close().catch(() => undefined)
      facts.mcpStderr = logs.join("")
    }
  } finally {
    if (KEEP) {
      emit(`      kept: ${out}`)
      facts.keptAt = out
    } else {
      await rm(work, { recursive: true, force: true })
    }
  }

  return 0
}

const report = async (outcome: number, fatal?: Error): Promise<number> => {
  const pass = checks.filter((entry) => entry.status === "PASS").length
  const fail = checks.filter((entry) => entry.status === "FAIL").length
  const gaps = checks.filter((entry) => entry.status === "GAP").length
  const code = gaps > 0 ? 2 : fail > 0 || outcome !== 0 ? 1 : 0
  facts.totalMs = Date.now() - STARTED
  emit("")
  emit(
    code === 0
      ? `== done: ${pass}/${checks.length} checks passed in ${(facts.totalMs as number / 1000).toFixed(0)} s`
      : code === 2
        ? `== gap: ${pass} passed, ${gaps} could not run, ${fail} failed — exit 2 is a coverage gap, not a pass`
        : `== failed: ${fail} of ${checks.length} checks — ${fatal?.message.split("\n")[0] ?? "unknown"}`,
  )
  await Bun.write(join(EVIDENCE, "summary.txt"), `${lines.join("\n")}\n`).catch(() => undefined)
  await Bun.write(join(EVIDENCE, "evidence.json"), JSON.stringify({ run: RUN, code, checks, facts }, null, 2)).catch(
    () => undefined,
  )
  return code
}

let outcome = 0
let fatal: Error | undefined
try {
  outcome = await main()
} catch (error) {
  fatal = error instanceof Error ? error : new Error(String(error))
}
process.exit(await report(outcome, fatal))
