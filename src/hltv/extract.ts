/**
 * Readers that run *inside the page* (and inside a fixture page in tests). Playwright serializes each
 * function, so they may not close over anything: no imports, no module state. The selectors live only
 * here, so an HLTV markup change breaks one test file instead of production.
 */

export type RawMatch = {
  href: string
  teams: [string, string]
  scores: [number, number] | null
  event: string | null
  heading: string | null
}

export type RawMatchPage = {
  title: string
  demoHref: string | null
  date: string | null
  event: string | null
}

/** Every match row on a results page, in page order (hltv.org lists newest first). */
export const readMatches = (): RawMatch[] => {
  const text = (node: Element | null | undefined): string => (node?.textContent ?? "").replace(/\s+/g, " ").trim()
  const rows = Array.from(document.querySelectorAll(".result-con"))
  const out: RawMatch[] = []
  for (const row of rows) {
    const href = row.querySelector('a[href^="/matches/"]')?.getAttribute("href")
    if (!href) continue
    const teams = Array.from(row.querySelectorAll("td.team-cell .team")).map((cell) => text(cell))
    const scores = Array.from(row.querySelectorAll("td.result-score span")).map((cell) => Number(text(cell)))
    const event =
      text(row.querySelector("td.event .event-name")) ||
      row.querySelector("td.event img")?.getAttribute("title") ||
      null
    out.push({
      href,
      teams: [teams[0] ?? "", teams[1] ?? ""],
      scores: scores.length === 2 && scores.every(Number.isFinite) ? [scores[0]!, scores[1]!] : null,
      event,
      heading: text(row.closest(".results-sublist")?.querySelector(".standard-headline")) || null,
    })
  }
  return out
}

/** The match page's demo link, if it has one. */
export const readMatchPage = (): RawMatchPage => {
  const demo =
    document.querySelector('a[href^="/download/demo/"]')?.getAttribute("href") ??
    document.querySelector("[data-demo-link]")?.getAttribute("data-demo-link") ??
    null
  const dateNode = document.querySelector(".timeAndEvent .date")
  const unix = dateNode?.getAttribute("data-unix")
  const eventNode = document.querySelector(".timeAndEvent .event a") ?? document.querySelector(".timeAndEvent .event")
  return {
    title: document.title,
    demoHref: demo,
    date: unix && /^\d+$/.test(unix) ? new Date(Number(unix)).toISOString().slice(0, 10) : null,
    event: (eventNode?.textContent ?? "").replace(/\s+/g, " ").trim() || null,
  }
}
