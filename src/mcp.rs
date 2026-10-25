//! MCP server (stdio) over the same operations the CLI uses: `search`, `resolve`, `get`, and the
//! download manifest. The only difference between this and the CLI is the transport and the shape
//! of the arguments; both go through `Source` and `hltv::*`.
//!
//! It speaks newline-delimited JSON-RPC on stdin/stdout; progress and notes go to stderr, which MCP
//! clients show as server logs.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::browser::Chrome;
use crate::core::errors::HltvError;
use crate::core::manifest;
use crate::core::types::{Archive, DemoRef, ManifestEntry, Match, SearchFilter};
use crate::core::url::{classify, map_filter};
use crate::hltv::get::{Options, canonical_out, default_out, get};
use crate::hltv::resolve::resolve;
use crate::hltv::search::search;
use crate::process;
use crate::source::{Opener, with_source};

/// Every tool failure reaches the client as an explanation plus something to do about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Failure {
    pub message: String,
    pub remediation: String,
}

impl From<HltvError> for Failure {
    fn from(error: HltvError) -> Self {
        Self {
            message: error.to_string(),
            remediation: error.remediation(),
        }
    }
}

fn fail(error: HltvError) -> Json<Failure> {
    Json(error.into())
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// HLTV team id, e.g. 9565 for Vitality
    pub team: Option<u64>,
    /// map name, e.g. cache or de_cache
    pub map: Option<String>,
    /// max matches, default 1, max 100
    pub limit: Option<i64>,
    /// results to skip, for paging
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolveParams {
    /// a match URL, e.g. `https://www.hltv.org/matches/2399723/vitality-vs-9z`
    pub match_url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadParams {
    /// match, results, or demo URL on hltv.org
    pub url: String,
    /// destination folder (default: ~/.cache/hltv-demo/archives)
    pub out: Option<String>,
    /// matches a results URL contributes, default 1
    pub limit: Option<i64>,
    /// pause between matches in ms, default 2000
    pub delay_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListParams {
    /// folder holding .hltv-demo.tsv
    pub out: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Listing {
    pub manifest: String,
    pub entries: Vec<ManifestEntry>,
}

/// The server, generic over the browser opener: tests pass one backed by a fake Source, and `main`
/// passes the real one.
#[derive(Clone)]
pub struct HltvServer<O: Opener> {
    opener: Arc<O>,
    tool_router: ToolRouter<Self>,
}

/// A client may send any number. Below zero reads as zero; the operations own the limits (`search`
/// clamps a limit to 1..=100, an offset of 0 is no offset, a delay of 0 is no pause).
fn whole(n: i64) -> u64 {
    n.max(0) as u64
}

fn count(n: i64) -> usize {
    usize::try_from(whole(n)).unwrap_or(usize::MAX)
}

#[tool_router]
impl<O: Opener> HltvServer<O> {
    pub fn new(opener: O) -> Self {
        Self {
            opener: Arc::new(opener),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List recent HLTV matches, newest first, optionally filtered by team and map. Reads a results page; it does not say whether a match has a demo (use resolve_demo)."
    )]
    pub async fn search_matches(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<Json<Vec<Match>>, Json<Failure>> {
        // Validate at the boundary: a bad map must fail instantly, not after launching Chrome.
        if let Some(map) = &params.map {
            map_filter(map).map_err(fail)?;
        }
        let filter = SearchFilter {
            team: params.team,
            map: params.map,
            limit: params.limit.map(count),
            offset: params.offset.map(whole),
        };
        with_source(&*self.opener, async |source| search(source, &filter).await)
            .await
            .map(Json)
            .map_err(fail)
    }

    #[tool(
        description = "Find the demo archive URL of one match page. Fails when the match publishes no demo."
    )]
    pub async fn resolve_demo(
        &self,
        Parameters(params): Parameters<ResolveParams>,
    ) -> Result<Json<DemoRef>, Json<Failure>> {
        classify(&params.match_url).map_err(fail)?;
        with_source(&*self.opener, async |source| {
            resolve(source, &params.match_url).await
        })
        .await
        .map(Json)
        .map_err(fail)
    }

    #[tool(
        description = "Download the demo archive for a match, results, or /download/demo/<id> URL into a folder. One archive holds every map of the series. Already-downloaded archives are skipped. Fails only when nothing could be downloaded."
    )]
    pub async fn download_demo(
        &self,
        Parameters(params): Parameters<DownloadParams>,
    ) -> Result<Json<Vec<Archive>>, Json<Failure>> {
        classify(&params.url).map_err(fail)?;
        let on_note = |text: &str| eprintln!("{text}");
        let options = Options {
            out: params.out.map(PathBuf::from),
            limit: params.limit.map(count),
            delay: params.delay_ms.map(|ms| Duration::from_millis(whole(ms))),
            on_note: Some(&on_note),
        };
        let archives = with_source(&*self.opener, async |source| {
            get(source, &params.url, &options).await
        })
        .await
        .map_err(fail)?;
        if archives.is_empty() {
            return Err(Json(Failure {
                message: "nothing downloaded".into(),
                remediation: "check the URL in a browser; the run recorded no archive".into(),
            }));
        }
        Ok(Json(archives))
    }

    #[tool(
        description = "List archives recorded in a folder's download manifest, with the demo URL each came from."
    )]
    pub async fn list_downloads(
        &self,
        Parameters(params): Parameters<ListParams>,
    ) -> Result<Json<Listing>, Json<Failure>> {
        let out = canonical_out(params.out.as_deref().map(std::path::Path::new));
        let entries = manifest::read(&out).map_err(fail)?;
        Ok(Json(Listing {
            manifest: manifest::manifest_path(&out).to_string_lossy().into_owned(),
            entries,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl<O: Opener> ServerHandler for HltvServer<O> {
    fn get_info(&self) -> ServerConfig {
        let instructions = [
            "Find and download CS2 demo archives from hltv.org through the user's own Chrome.",
            "search_matches → resolve_demo → download_demo is the usual order.",
            "HLTV challenges non-browser clients and sometimes challenges this browser too: when a tool fails",
            "with 'behind the Cloudflare challenge', tell the user to answer it once in the printed Chrome",
            "profile, then retry. Demos are 440-890 MB each; download one match unless asked for more.",
            &format!("Archives and {} live in {} unless the call passes out.", manifest::MANIFEST, default_out().display()),
        ]
        .join(" ");
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("hltv-demo", env!("CARGO_PKG_VERSION")))
            .with_instructions(instructions)
    }
}

/// Serve until the client disconnects or a signal arrives. Returning drops the runtime, and with it
/// any tool call still running, so its Chrome is closed and the profile lock released.
pub async fn main() -> ExitCode {
    process::init_logging();
    let server = tokio::select! {
        served = HltvServer::new(Chrome).serve(rmcp::transport::stdio()) => served,
        signal = process::shutdown() => return ExitCode::from(signal.exit_code()),
    };
    let server = match server {
        Ok(server) => server,
        Err(error) => {
            eprintln!("hltv-demo-mcp: {error}");
            return ExitCode::from(1);
        }
    };
    tokio::select! {
        done = server.waiting() => match done {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("hltv-demo-mcp: {error}");
                ExitCode::from(1)
            }
        },
        signal = process::shutdown() => {
            eprintln!("hltv-demo-mcp: stopped by {signal}");
            ExitCode::from(signal.exit_code())
        }
    }
}
