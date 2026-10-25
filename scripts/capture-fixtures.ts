#!/usr/bin/env bun
/**
 * Save real hltv.org HTML so the extractors can be tested without the network — trimmed to the
 * subtrees the extractors read, never a page dump. Trimming is part of capture, not a cleanup step
 * somebody remembers: what lands in `test/fixtures/` is already pruned, and `trim()` refuses to write
 * a file that still carries a script, a tracker, or a per-visitor attribute.
 *
 * usage: bun run scripts/capture-fixtures.ts           re-capture (headed Chrome; answers Cloudflare if asked)
 *        bun run scripts/capture-fixtures.ts --trim    re-trim the saved fixtures (offline, headless)
 */
import { mkdir, readFile, writeFile } from "node:fs/promises"
import { homedir } from "node:os"
import { join } from "node:path"
import { chromium, type BrowserContext, type Page } from "playwright-core"
import { chromePath } from "../src/browser.ts"

const DIR = new URL("../test/fixtures/", import.meta.url).pathname
/** What a fixture must still contain after trimming, so a challenge page can never overwrite one. */
const PAGES: [string, string, RegExp][] = [
  ["results-cache", "https://www.hltv.org/results?team=9565&map=de_cache", /class="result-con/],
  ["match", "https://www.hltv.org/matches/2399723/vitality-vs-9z-pgl-masters-bucharest-2026", /\/download\/demo\/\d+/],
]

/** Serialized into the page, so it may not close over anything: same rule as the extractors. */
const TRIM = (): void => {
  /** A node the extractors read: kept with its whole subtree. Everything above it survives as path. */
  const read = new Set<Element>()
  const mark = (node: Element | null): void => {
    if (node) read.add(node)
  }
  const roots = [
    ".results-sublist", // a date heading plus the rows under it
    ".result-con", // a row, including the featured block that has no heading
    ".timeAndEvent", // the match page's date and event
    ".streams", // where the demo link lives
    'a[href^="/download/demo/"]',
    "title",
  ]
  for (const selector of roots) for (const node of Array.from(document.querySelectorAll(selector))) mark(node)

  // A read node is kept whole: the extractors read the markup inside a row, so pruning stops there.
  // An ancestor is only a path to one, so its other children go.
  const prune = (node: Element): boolean => {
    let hasContent = false
    for (const child of Array.from(node.children)) {
      if (read.has(child)) {
        hasContent = true
        continue
      }
      if (prune(child)) hasContent = true
      else child.remove()
    }
    return hasContent
  }
  prune(document.documentElement)

  // Nothing executable and nothing that phones home survives the prune, but be explicit: a future
  // page may put a script inside a kept subtree.
  for (const node of Array.from(
    document.querySelectorAll("script, noscript, iframe, link, style, [data-cf-beacon], .pixel-tracking, img[src*='facebook.com/tr']"),
  )) {
    node.remove()
  }

  // Capture metadata hides in comments ("Country: US-IL", build ids); comments are not structure.
  const dropComments = (node: Node): void => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === Node.COMMENT_NODE) child.parentNode?.removeChild(child)
      else dropComments(child)
    }
  }
  dropComments(document)

  // Only structure survives, by explicit allow-list: no per-visitor attributes (one reader's country,
  // ad config), no inline style, no handlers. A new selector that needs another attribute fails the
  // fixture test, which is the point of having one.
  const STRUCTURAL =
    /^(class|href|src|alt|title|id|lang|colspan|rowspan|type|rel|charset|data-unix|data-demo-link|data-time-format)$/
  for (const node of [document.documentElement, ...Array.from(document.querySelectorAll("*"))]) {
    for (const attribute of Array.from(node.attributes)) if (!STRUCTURAL.test(attribute.name)) node.removeAttribute(attribute.name)
  }

  if (document.querySelector("meta[charset]") === null) {
    const meta = document.createElement("meta")
    meta.setAttribute("charset", "utf-8")
    document.head.prepend(meta)
  }
}

/** A trimmed fixture must not carry what it was trimmed to remove. */
const FORBIDDEN =
  /<script|<!--|facebook\.com\/tr|googletagmanager|doubleclick|cloudflareinsights|data-cf-beacon|data-client-country-iso|Country: [A-Z]{2}-[A-Z]{2}/

const trim = async (page: Page, name: string, anchor: RegExp): Promise<string> => {
  await page.evaluate(TRIM)
  // Belt and braces: hltv.org serves variants whose comments land inside a kept subtree (a <template>
  // or a slow ad slot), which the DOM walk cannot reach. Comments carry no structure, so drop any left.
  const html = (await page.content()).replace(/<!--[\s\S]*?-->/g, "")
  const leaked = FORBIDDEN.exec(html)
  if (leaked) {
    const at = Math.max(0, leaked.index - 120)
    throw new Error(`${name}: trim left ${leaked[0]} behind: …${html.slice(at, leaked.index + 160)}…`)
  }
  if (!anchor.test(html)) throw new Error(`${name}: trimmed page lost its anchor ${anchor}; not writing`)
  return html
}

const report = (name: string, before: number, after: number) =>
  console.error(`${name}: ${before} → ${after} bytes (${Math.round((100 * after) / before)}%), no scripts`)

const retrim = async (): Promise<void> => {
  const browser = await chromium.launch({ executablePath: chromePath(), headless: true })
  try {
    const page = await browser.newPage()
    for (const [name, , anchor] of PAGES) {
      const path = join(DIR, `${name}.html`)
      const original = await readFile(path, "utf8")
      await page.setContent(original)
      const html = await trim(page, name, anchor)
      await writeFile(path, html)
      report(name, original.length, html.length)
    }
  } finally {
    await browser.close()
  }
}

const capture = async (): Promise<void> => {
  const context: BrowserContext = await chromium.launchPersistentContext(join(homedir(), ".cache/hltv-demo/chrome"), {
    executablePath: chromePath(),
    headless: false,
    args: ["--disable-blink-features=AutomationControlled"],
  })
  try {
    for (const [name, url, anchor] of PAGES) {
      const page = await context.newPage()
      await page.goto(url, { waitUntil: "domcontentloaded", timeout: 60_000 }).catch(() => undefined)
      let title = ""
      for (let i = 0; i < 120; i++) {
        title = await page.title().catch(() => "")
        if (!/Just a moment|security verification/i.test(title)) break
        await new Promise((open) => setTimeout(open, 500))
      }
      if (/Just a moment|security verification/i.test(title)) {
        await page.close()
        throw new Error(
          `${name}: still behind the Cloudflare challenge (${title}); answer it once in the fetch profile ` +
            `(\`open -a "Google Chrome" --args --user-data-dir=$HOME/.cache/hltv-demo/chrome https://www.hltv.org\`), then rerun. Nothing was written.`,
        )
      }
      const original = (await page.content()).length
      const html = await trim(page, name, anchor)
      await mkdir(DIR, { recursive: true })
      await writeFile(join(DIR, `${name}.html`), html)
      report(name, original, html.length)
      await page.close()
    }
  } finally {
    await context.close()
  }
}

await (process.argv.includes("--trim") ? retrim() : capture())
