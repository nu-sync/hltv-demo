const MONTHS = [
  "january", "february", "march", "april", "may", "june",
  "july", "august", "september", "october", "november", "december",
]

/**
 * ISO day from a results-list heading such as "Results for October 24th 2026".
 * Returns null for anything else, so an unknown heading costs a date field, not a whole search.
 */
export const fromHeading = (heading: string | null | undefined): string | null => {
  if (!heading) return null
  const match = /([A-Za-z]+)\s+(\d{1,2})(?:st|nd|rd|th)?\s+(\d{4})/.exec(heading)
  if (!match) return null
  const month = MONTHS.indexOf(match[1]!.toLowerCase())
  if (month === -1) return null
  const day = Number(match[2])
  const year = Number(match[3])
  if (day < 1 || day > 31) return null
  return `${year}-${String(month + 1).padStart(2, "0")}-${String(day).padStart(2, "0")}`
}
