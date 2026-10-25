/**
 * The CLI as a user runs it: a subprocess, real exit codes, real output. Only the paths that fail
 * before a browser opens are here — anything that needs hltv.org lives in scripts/live-check.sh.
 */
import { describe, expect, test } from "bun:test"
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

const REPO = join(import.meta.dir, "..", "..")

const runCli = async (args: string[], env: Record<string, string> = {}) => {
  const proc = Bun.spawn(["bun", "run", join(REPO, "src", "cli.ts"), ...args], {
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, ...env },
  })
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()])
  return { code: await proc.exited, stdout, stderr }
}

describe("cli", () => {
  test("--help prints usage and exits 0", async () => {
    const { code, stdout } = await runCli(["--help"])
    expect(code).toBe(0)
    expect(stdout).toContain("hltv-demo search")
    expect(stdout).toContain("hltv-demo download <url>")
  })

  test("an unknown command is a usage error (2)", async () => {
    const { code, stderr } = await runCli(["frobnicate"])
    expect(code).toBe(2)
    expect(stderr).toContain("unknown command: frobnicate")
  })

  test("an unknown flag is a usage error (2)", async () => {
    expect((await runCli(["search", "--nope"])).code).toBe(2)
  })

  test("a flag without a value is a usage error (2)", async () => {
    const { code, stderr } = await runCli(["search", "--limit"])
    expect(code).toBe(2)
    expect(stderr).toContain("--limit needs a value")
  })

  test("a limit above 100 is a usage error (2), pointing at --offset", async () => {
    const { code, stderr } = await runCli(["search", "--limit", "500"])
    expect(code).toBe(2)
    expect(stderr).toContain("between 1 and 100")
    expect(stderr).toContain("--offset")
  })

  test("a non-numeric team id is a usage error (2)", async () => {
    expect((await runCli(["search", "--team", "vitality"])).code).toBe(2)
  })

  test("resolve without a URL is a usage error (2)", async () => {
    const { code, stderr } = await runCli(["resolve"])
    expect(code).toBe(2)
    expect(stderr).toContain("resolve needs a URL")
  })

  test("a URL off hltv.org fails fast (1) with a remediation, without opening a browser", async () => {
    const { code, stderr } = await runCli(["download", "https://example.com/matches/1/x"])
    expect(code).toBe(1)
    expect(stderr).toContain("is not an hltv.org URL")
    expect(stderr).toContain("/download/demo/")
  })

  test("--headless reaches the browser launch, so the challenge it meets is legible", async () => {
    const dir = await mkdtemp(join(tmpdir(), "hltv-demo-cli-"))
    const args = join(dir, "args")
    // A stand-in Chrome that records how it was launched and exits, so no browser ever opens.
    const chrome = join(dir, "chrome")
    await writeFile(chrome, `#!/bin/sh\necho "$@" > "${args}"\nexit 1\n`, { mode: 0o755 })
    await mkdir(join(dir, "profile"))
    const env = { CHROME: chrome, HLTV_DEMO_PROFILE: join(dir, "profile", "chrome") }

    await runCli(["search"], env)
    expect(await readFile(args, "utf8")).not.toContain("--headless")
    await runCli(["search", "--headless"], env)
    expect(await readFile(args, "utf8")).toContain("--headless")
  })

  test("an hltv.org page we do not read is a domain failure (1)", async () => {
    const { code, stderr } = await runCli(["resolve", "https://www.hltv.org/player/7998/s1mple"])
    expect(code).toBe(1)
    expect(stderr).toContain("is not a match, results, or demo URL")
  })
})
