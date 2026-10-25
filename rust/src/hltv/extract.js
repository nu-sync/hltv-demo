(() => {
  const text = (node) => (node?.textContent ?? "").replace(/\s+/g, " ").trim()
  return {
    readMatches() {
      const out = []
      for (const row of document.querySelectorAll(".result-con")) {
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
          scores: scores.length === 2 && scores.every((s) => Number.isInteger(s) && s >= 0) ? scores : null,
          event,
          heading: text(row.closest(".results-sublist")?.querySelector(".standard-headline")) || null,
        })
      }
      return out
    },
    readMatchPage() {
      const demo =
        document.querySelector('a[href^="/download/demo/"]')?.getAttribute("href") ??
        document.querySelector("[data-demo-link]")?.getAttribute("data-demo-link") ??
        null
      const unix = document.querySelector(".timeAndEvent .date")?.getAttribute("data-unix")
      const eventNode =
        document.querySelector(".timeAndEvent .event a") ?? document.querySelector(".timeAndEvent .event")
      return {
        title: document.title,
        demoHref: demo,
        date: unix && /^\d+$/.test(unix) ? new Date(Number(unix)).toISOString().slice(0, 10) : null,
        event: text(eventNode) || null,
      }
    },
  }
})()
