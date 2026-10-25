// Save real hltv.org HTML so the extractors can be tested without the network.
// usage: bun run scripts/capture-fixtures.ts [url ...]   (headed Chrome; answers Cloudflare if asked)
import { chromium } from "playwright-core"
import { mkdir, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"

const dir = new URL("../test/fixtures/", import.meta.url).pathname
await mkdir(dir, { recursive: true })

const browser = await chromium.launchPersistentContext(join(homedir(), ".cache/hltv-demo/chrome"), {
  channel: "chrome",
  headless: false,
  args: ["--disable-blink-features=AutomationControlled"],
})

const pages: [string, string][] = [
  ["results-cache", "https://www.hltv.org/results?team=9565&map=de_cache"],
  ["match", "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026"],
]

for (const [name, url] of pages) {
  const page = await browser.newPage()
  await page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => {})
  for (let i = 0; i < 120; i++) {
    const title = await page.title().catch(() => "")
    if (!/Just a moment|security verification/i.test(title)) break
    await new Promise((r) => setTimeout(r, 500))
  }
  const html = await page.content()
  await writeFile(join(dir, `${name}.html`), html)
  console.error(`${name}: ${html.length} bytes, title=${JSON.stringify(await page.title())}`)
  await page.close()
}
await browser.close()
