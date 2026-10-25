#!/usr/bin/env bun
/**
 * MCP server (stdio) over the same operations the CLI uses: `search`, `resolve`, `get`, and the
 * download manifest. The only difference between this and the CLI is the transport and the shape of
 * the arguments — both go through `Source` and `src/hltv/*`.
 *
 * Launch: `bun run src/mcp.ts` (or the `hltv-demo-mcp` bin). It speaks newline-delimited JSON-RPC on
 * stdin/stdout; progress and notes go to stderr, which MCP clients show as server logs.
 */
import { Effect, Layer, Schema } from "effect"
import * as McpProtocol from "effect/ai/McpProtocol"
import * as McpServer from "effect/ai/McpServer"
import { Tool, Toolkit } from "effect/ai"
import { BunRuntime, BunStdio } from "@effect/platform-bun"
import { resolve as absolutePath } from "node:path"
import { explain } from "./core/errors.ts"
import * as manifest from "./core/manifest.ts"
import {
  ArchiveSchema,
  DemoRefSchema,
  ManifestEntrySchema,
  MatchSchema,
  type Archive,
  type DemoRef,
  type Match,
} from "./core/types.ts"
import { classifyOrFail } from "./classify.ts"
import { withSource as realWithSource, type WithSource } from "./source.ts"
import { defaultOut, get } from "./hltv/get.ts"
import { resolve } from "./hltv/resolve.ts"
import { MAX_LIMIT, search } from "./hltv/search.ts"

const VERSION = "0.1.0"

/** Every tool failure reaches the client as an explanation plus something to do about it. */
const FailureSchema = Schema.Struct({ message: Schema.String, remediation: Schema.String })

/** Every HltvError carries both fields already; the client gets exactly those. */
const toFailure = (error: { readonly message: string; readonly remediation: string }) => ({
  message: error.message,
  remediation: error.remediation,
})

export const tools = Toolkit.make(
  Tool.make("search_matches", {
    description:
      "List recent HLTV matches, newest first, optionally filtered by team and map. Reads a results " +
      "page; it does not say whether a match has a demo (use resolve_demo).",
    parameters: Schema.Struct({
      team: Schema.optional(Schema.Number.annotate({ description: "HLTV team id, e.g. 9565 for Vitality" })),
      map: Schema.optional(Schema.String.annotate({ description: "map name, e.g. cache or de_cache" })),
      limit: Schema.optional(Schema.Number.annotate({ description: `max matches, default 1, max ${MAX_LIMIT}` })),
      offset: Schema.optional(Schema.Number.annotate({ description: "results to skip, for paging" })),
    }),
    success: Schema.Array(MatchSchema),
    failure: FailureSchema,
    failureMode: "return",
  }),
  Tool.make("resolve_demo", {
    description: "Find the demo archive URL of one match page. Fails when the match publishes no demo.",
    parameters: Schema.Struct({
      matchUrl: Schema.String.annotate({ description: "an hltv.org/matches/<id>/... URL" }),
    }),
    success: DemoRefSchema,
    failure: FailureSchema,
    failureMode: "return",
  }),
  Tool.make("download_demo", {
    description:
      "Download the demo archive for a match, results, or /download/demo/<id> URL into a folder. One " +
      "archive holds every map of the series. Already-downloaded archives are skipped. Fails only when " +
      "nothing could be downloaded.",
    parameters: Schema.Struct({
      url: Schema.String.annotate({ description: "match, results, or demo URL on hltv.org" }),
      out: Schema.optional(Schema.String.annotate({ description: "destination folder (default: ~/.cache/hltv-demo/archives)" })),
      limit: Schema.optional(Schema.Number.annotate({ description: "matches a results URL contributes, default 1" })),
      delayMs: Schema.optional(Schema.Number.annotate({ description: "pause between matches in ms, default 2000" })),
    }),
    success: Schema.Array(ArchiveSchema),
    failure: FailureSchema,
    failureMode: "return",
  }),
  Tool.make("list_downloads", {
    description: "List archives recorded in a folder's download manifest, with the demo URL each came from.",
    parameters: Schema.Struct({
      out: Schema.optional(Schema.String.annotate({ description: "folder holding .hltv-demo.tsv" })),
    }),
    success: Schema.Struct({
      manifest: Schema.String,
      entries: Schema.Array(ManifestEntrySchema),
    }),
    failure: FailureSchema,
    failureMode: "return",
  }),
)

/**
 * Tool handlers, as a factory over the browser runner: tests pass one backed by a fake Source, and the
 * server below passes the real one.
 */
export const makeHandlers = (withSource: WithSource) => ({
  search_matches: (params: { team?: number; map?: string; limit?: number; offset?: number }) =>
    withSource((source) => search(source, params)).pipe(Effect.mapError(toFailure)),
  resolve_demo: (params: { matchUrl: string }) =>
    Effect.gen(function* () {
      // Validate at the boundary: a bad URL must fail instantly, not after launching Chrome.
      yield* classifyOrFail(params.matchUrl)
      return yield* withSource((source) => resolve(source, params.matchUrl))
    }).pipe(Effect.mapError(toFailure)),
  download_demo: (params: { url: string; out?: string; limit?: number; delayMs?: number }) =>
    Effect.gen(function* () {
      yield* classifyOrFail(params.url)
      const archives = yield* withSource((source) =>
        get(source, params.url, {
          out: params.out,
          limit: params.limit,
          delayMs: params.delayMs,
          onNote: (text) => console.error(text),
        }),
      )
      if (archives.length === 0) {
        return yield* Effect.fail({
          message: "nothing downloaded",
          remediation: "check the URL in a browser; the run recorded no archive",
        })
      }
      return archives
    }).pipe(Effect.mapError(toFailure)),
  list_downloads: (params: { out?: string }) =>
    Effect.gen(function* () {
      const out = absolutePath(params.out ?? defaultOut())
      const entries = yield* manifest.attempt(out, () => manifest.read(out))
      return { manifest: manifest.manifestPath(out), entries }
    }).pipe(Effect.mapError(toFailure)),
})

export const handlers = makeHandlers(realWithSource)

export const McpTools = McpServer.toolkit(tools).pipe(Layer.provide(tools.toLayer(handlers)))

export const Server = McpServer.layerStdio({
  name: "hltv-demo",
  version: VERSION,
  instructions: [
    "Find and download CS2 demo archives from hltv.org through the user's own Chrome.",
    "search_matches → resolve_demo → download_demo is the usual order.",
    "HLTV challenges non-browser clients and sometimes challenges this browser too: when a tool fails",
    "with 'behind the Cloudflare challenge', tell the user to answer it once in the printed Chrome",
    "profile, then retry. Demos are 440-890 MB each; download one match unless asked for more.",
    `Archives and ${manifest.MANIFEST} live in ${defaultOut()} unless the call passes out.`,
  ].join(" "),
  protocols: [McpProtocol.v2025_06_18],
}).pipe(Layer.provide(McpTools), Layer.provide(BunStdio.layer))

if (import.meta.main) {
  BunRuntime.runMain(Layer.launch(Server))
}

export type { Archive, DemoRef, Match }
export { defaultOut, explain }
