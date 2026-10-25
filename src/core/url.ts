import { NotHltvUrl, UnsupportedPage } from "./errors.ts"

export const BASE = "https://www.hltv.org"

export type Target =
  | { kind: "match"; matchId: number; url: string }
  | { kind: "results"; url: string }
  | { kind: "demo"; demoId: number; url: string }

/** Path and query of an hltv.org URL, or of a bare path. Throws NotHltvUrl for any other host. */
export const pathOf = (url: string): string => {
  if (url.startsWith("/")) return url
  const rest = url.replace(/^https?:\/\//, "")
  const slash = rest.indexOf("/")
  const host = (slash === -1 ? rest : rest.slice(0, slash)).toLowerCase()
  if (host !== "hltv.org" && host !== "www.hltv.org") throw new NotHltvUrl({ url })
  return slash === -1 ? "/" : rest.slice(slash)
}

/** Absolute hltv.org URL, so a bare path and a full URL behave the same. */
export const absolute = (url: string): string => `${BASE}${pathOf(url)}`

/** Identifies the page a URL opens: a match or demo by id (the slug is not identity), else path+query. */
export const pageKey = (url: string): string => {
  let path: string
  try {
    path = pathOf(url)
  } catch {
    return url
  }
  const segments = path.split("/").filter((s) => s.length > 0)
  if (segments[0] === "matches" && segments[1]) return `/matches/${segments[1]}`
  if (segments[0] === "download" && segments[1] === "demo" && segments[2]) return `/download/demo/${segments[2]}`
  return path
}

/** Read a URL as one of the three shapes this tool understands. */
export const classify = (url: string): Target => {
  const path = pathOf(url)
  const key = pageKey(url)
  if (key.startsWith("/matches/")) {
    return { kind: "match", matchId: Number(key.slice("/matches/".length)), url: `${BASE}${path}` }
  }
  if (key.startsWith("/download/demo/")) {
    return { kind: "demo", demoId: Number(key.slice("/download/demo/".length)), url: `${BASE}${key}` }
  }
  if (path === "/results" || path.startsWith("/results?")) return { kind: "results", url: `${BASE}${path}` }
  throw new UnsupportedPage({ url })
}

/** hltv.org map filter with the `de_` prefix HLTV expects. */
export const mapFilter = (map: string): string => {
  const name = map.trim().toLowerCase()
  if (!/^[a-z0-9_]+$/.test(name)) throw new Error(`not a map name: ${map}`)
  return name.startsWith("de_") ? name : `de_${name}`
}

/** The filter a results URL already carries, so a URL is never silently widened. */
export const resultsFilter = (url: string): { team?: number; map?: string; offset?: number } => {
  const query = pathOf(url).split("?")[1] ?? ""
  const params = new URLSearchParams(query)
  const team = Number(params.get("team"))
  const offset = Number(params.get("offset"))
  return {
    team: Number.isInteger(team) && team > 0 ? team : undefined,
    map: params.get("map") ?? undefined,
    offset: Number.isInteger(offset) && offset > 0 ? offset : undefined,
  }
}

/** A results URL for a team and/or map, paged by offset. */
export const resultsUrl = (options: { team?: number; map?: string; offset?: number }): string => {
  const params = new URLSearchParams()
  if (options.team !== undefined) params.set("team", String(options.team))
  if (options.map !== undefined) params.set("map", mapFilter(options.map))
  if (options.offset) params.set("offset", String(options.offset))
  const query = params.toString()
  return `${BASE}/results${query ? `?${query}` : ""}`
}
