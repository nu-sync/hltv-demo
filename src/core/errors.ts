import { Data } from "effect"

/**
 * Every failure the tool can produce, with the remediation printed by the CLI. Each carries the values
 * a user needs to act, so no message has to be reconstructed at the call site.
 */
export type HltvError =
  | NotHltvUrl
  | UnsupportedPage
  | Challenge
  | NoDemoFound
  | NoMatchesFound
  | DownloadFailed
  | BrowserMissing
  | BrowserFailed
  | ProfileBusy
  | ManifestUnreadable

/** Not an hltv.org URL. Nothing was opened. */
export class NotHltvUrl extends Data.TaggedError("NotHltvUrl")<{ url: string }> {
  override get message() {
    return `${this.url} is not an hltv.org URL`
  }
  get remediation() {
    return "pass https://www.hltv.org/... or a bare path such as /download/demo/113159"
  }
}

/** hltv.org, but not a page this tool reads. */
export class UnsupportedPage extends Data.TaggedError("UnsupportedPage")<{ url: string }> {
  override get message() {
    return `${this.url} is not a match, results, or demo URL`
  }
  get remediation() {
    return "accepted: /matches/<id>/..., /results?..., /download/demo/<id>"
  }
}

/** Cloudflare answered with its challenge instead of the page. Only a person can clear it. */
export class Challenge extends Data.TaggedError("Challenge")<{ url: string; profile: string }> {
  override get message() {
    return `${this.url} is behind the Cloudflare challenge`
  }
  get remediation() {
    return [
      "answer it once in the fetch profile, then run the same command again:",
      `  open -a "Google Chrome" --args --user-data-dir=${this.profile} https://www.hltv.org`,
      "headless Chrome cannot pass it; do not add a bypass",
    ].join("\n")
  }
}

/** The match page loaded but offers no demo. */
export class NoDemoFound extends Data.TaggedError("NoDemoFound")<{ url: string }> {
  override get message() {
    return `${this.url} has no demo download link`
  }
  get remediation() {
    return "the match may not publish a demo (online or early-stage), or HLTV changed its markup"
  }
}

/** A results page listed nothing for the filter. */
export class NoMatchesFound extends Data.TaggedError("NoMatchesFound")<{ url: string }> {
  override get message() {
    return `${this.url} lists no matches`
  }
  get remediation() {
    return "check the team id and map name, or drop the filter"
  }
}

/** A download was canceled, incomplete, or stalled. */
export class DownloadFailed extends Data.TaggedError("DownloadFailed")<{
  url: string
  reason: string
  bytes?: number
}> {
  override get message() {
    const size = this.bytes === undefined ? "" : ` after ${Math.round(this.bytes / 1e6)} MB`
    return `${this.url} did not download${size}: ${this.reason}`
  }
  get remediation() {
    return "retry; nothing partial is recorded, so a retry is always safe"
  }
}

/** No Chrome to drive. */
export class BrowserMissing extends Data.TaggedError("BrowserMissing")<{ tried: string }> {
  override get message() {
    return `no Chrome found at ${this.tried}`
  }
  get remediation() {
    return "install Google Chrome, or set CHROME=/path/to/chrome"
  }
}

/** A browser operation failed: a tab would not open, or the page threw. */
export class BrowserFailed extends Data.TaggedError("BrowserFailed")<{ reason: string }> {
  override get message() {
    return `browser operation failed: ${this.reason}`
  }
  get remediation() {
    return "close the Chrome window the tool opened and retry; a killed run can leave the profile held"
  }
}

/** Another process is driving the shared Chrome profile. */
export class ProfileBusy extends Data.TaggedError("ProfileBusy")<{ profile: string; pid?: number }> {
  override get message() {
    return `the Chrome profile ${this.profile} is in use${this.pid ? ` by pid ${this.pid}` : ""}`
  }
  get remediation() {
    return "wait for the other run to finish, or use HLTV_DEMO_PROFILE=/another/path to isolate it"
  }
}

/** The download manifest could not be read. */
export class ManifestUnreadable extends Data.TaggedError("ManifestUnreadable")<{
  path: string
  reason: string
}> {
  override get message() {
    return `cannot read the download manifest ${this.path}: ${this.reason}`
  }
  get remediation() {
    return "delete the manifest to start a fresh one; archives already on disk are not re-downloaded by name"
  }
}

/** One line per failure, for humans. */
export const explain = (error: HltvError): string =>
  [error.message, "  " + error.remediation.replaceAll("\n", "\n  ")].join("\n")
