//! MCP server exposing Jidr to AI agents (`jidr mcp`, stdio).
//!
//! Generic tools cover every protocol command (`command_list`,
//! `command_run`, `jobs_*`); typed tools cover the hot paths of driving a
//! device (`screenshot`, `tap`, `swipe`, `type_text`, `press_key`, `ui_dump`,
//! `logcat_tail`). Destructive commands follow D5.

pub mod device;

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use jidr_client::Client;
use jidr_core::provider::CommandSpec;
use jidr_core::{ConnectInfo, Device, JidrError, JidrResult};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::DeviceIo;

/// Methods that only make sense on a raw protocol connection.
const NOT_CALLABLE: &[&str] = &[
    "hello",
    "events.subscribe",
    "logcat.follow",
    "daemon.shutdown",
];
const DEFAULT_RUN_TIMEOUT_S: u64 = 900;
const UI_DUMP_LIMIT: usize = 400_000;

#[derive(Debug, Clone, Default)]
pub struct McpOptions {
    /// Allow destructive commands on devices the agent doesn't own (D5).
    pub allow_destructive: bool,
}

pub struct JidrMcp {
    client: Arc<Client>,
    opts: McpOptions,
}

impl std::fmt::Debug for JidrMcp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JidrMcp")
            .field("opts", &self.opts)
            .finish_non_exhaustive()
    }
}

/// Serves MCP on stdin/stdout until the client disconnects.
pub async fn serve_stdio(client: Client, opts: McpOptions) -> JidrResult<()> {
    let service = JidrMcp::new(client, opts)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| JidrError::unexpected(format!("MCP initialize failed: {e}")))?;
    service
        .waiting()
        .await
        .map_err(|e| JidrError::unexpected(format!("MCP server: {e}")))?;
    Ok(())
}

// ---- tool params --------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandListArgs {
    /// Only commands whose id starts with this, e.g. `device.`.
    #[serde(default)]
    prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommandRunArgs {
    /// Command id from command_list, e.g. `device.start`.
    command: String,
    /// Params object matching the command's schema.
    #[serde(default)]
    params: Option<Value>,
    /// For long-running commands: wait at most this long (default 900 s),
    /// then return the job so it can be checked with jobs_list.
    #[serde(default)]
    timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct JobsListArgs {
    /// Include finished jobs.
    #[serde(default)]
    all: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct JobArgs {
    /// Job id.
    job: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeviceArgs {
    /// Device name or id.
    device: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Coords {
    /// Pixels of the screenshot tool's image (default).
    #[default]
    Screenshot,
    /// Raw device pixels.
    Device,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TapArgs {
    device: String,
    x: f64,
    y: f64,
    /// Coordinate space of x/y (default: the screenshot image).
    #[serde(default)]
    coords: Coords,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SwipeArgs {
    device: String,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    /// Gesture duration in ms (default 300).
    #[serde(default)]
    duration_ms: Option<u32>,
    #[serde(default)]
    coords: Coords,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TypeArgs {
    device: String,
    /// Text to type into the focused field (ASCII is reliable).
    text: String,
    /// Press Enter afterwards.
    #[serde(default)]
    enter: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeyArgs {
    device: String,
    /// `GoHome`, `GoBack`, `AppSwitch`, `Enter`, `Power`, `AudioVolumeUp`,
    /// `AudioVolumeDown`, `Tab`, `Backspace`, `ArrowUp`/`Down`/`Left`/`Right`.
    key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LogcatArgs {
    device: String,
    /// Number of recent lines (default 200).
    #[serde(default)]
    lines: Option<u32>,
    /// logcat filter specs, e.g. `["ActivityManager:I", "*:S"]`.
    #[serde(default)]
    filter: Vec<String>,
}

fn schema<T: JsonSchema>() -> JsonObject {
    let mut v = serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| json!({ "type": "object" }));
    if let Some(o) = v.as_object_mut() {
        o.remove("$schema");
        o.remove("title");
    }
    match v {
        Value::Object(o) => o,
        _ => JsonObject::new(),
    }
}

fn tool<T: JsonSchema>(name: &'static str, desc: &'static str, ann: ToolAnnotations) -> Tool {
    Tool::new(name, desc, Arc::new(schema::<T>())).with_annotations(ann)
}

fn read_only() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(true)
        .destructive(false)
        .idempotent(true)
        .open_world(false)
}

fn interacts() -> ToolAnnotations {
    ToolAnnotations::new()
        .read_only(false)
        .destructive(false)
        .idempotent(false)
        .open_world(false)
}

fn parse<T: serde::de::DeserializeOwned>(args: Option<JsonObject>) -> JidrResult<T> {
    jidr_core::provider::parse_params(Value::Object(args.unwrap_or_default()))
}

/// A Jidr error as a tool result the agent can read and act on.
fn tool_error(e: &JidrError) -> CallToolResult {
    let mut r = CallToolResult::error(vec![ContentBlock::text(format!(
        "{}: {}",
        e.code, e.message
    ))]);
    r.structured_content = Some(json!({ "code": e.code, "message": e.message, "data": e.data }));
    r
}

fn ok_json(v: Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
    let mut r = CallToolResult::success(vec![ContentBlock::text(text)]);
    r.structured_content = Some(if v.is_object() {
        v
    } else {
        json!({ "result": v })
    });
    r
}

impl JidrMcp {
    pub fn new(client: Client, opts: McpOptions) -> Self {
        Self {
            client: Arc::new(client),
            opts,
        }
    }

    pub fn tools() -> Vec<Tool> {
        vec![
            tool::<CommandListArgs>(
                "command_list",
                "List every Jidr command (devices, SDK, images, root, sensors, snapshots…) with its params schema and flags. Use with command_run.",
                read_only(),
            ),
            tool::<CommandRunArgs>(
                "command_run",
                "Run any Jidr command by id, e.g. {\"command\":\"device.start\",\"params\":{\"device\":\"Phone\"}}. Long-running commands are waited for. Destructive commands only work on devices you created (agent-owned) unless the server allows them.",
                ToolAnnotations::new()
                    .read_only(false)
                    .destructive(true)
                    .idempotent(false)
                    .open_world(false),
            ),
            tool::<JobsListArgs>(
                "jobs_list",
                "Long-running operations (boots, installs) and their progress.",
                read_only(),
            ),
            tool::<JobArgs>(
                "jobs_cancel",
                "Cancel a running job; the device stays as it was before.",
                ToolAnnotations::new()
                    .read_only(false)
                    .destructive(false)
                    .idempotent(true)
                    .open_world(false),
            ),
            tool::<DeviceArgs>(
                "screenshot",
                "PNG of a running device's screen, fitted in 1024×1024. tap/swipe take coordinates in this image by default.",
                read_only(),
            ),
            tool::<TapArgs>("tap", "Tap a point on a running device.", interacts()),
            tool::<SwipeArgs>(
                "swipe",
                "Swipe/drag between two points (scroll, open the app drawer, …).",
                interacts(),
            ),
            tool::<TypeArgs>(
                "type_text",
                "Type text into the focused field of a running device.",
                interacts(),
            ),
            tool::<KeyArgs>(
                "press_key",
                "Press a device key: GoHome, GoBack, AppSwitch, Enter, Power, volume…",
                interacts(),
            ),
            tool::<DeviceArgs>(
                "ui_dump",
                "The on-screen UI hierarchy (uiautomator XML: text, resource-id, bounds) of a running device. Use bounds to find where to tap.",
                read_only(),
            ),
            tool::<LogcatArgs>(
                "logcat_tail",
                "Recent device log lines, optionally filtered.",
                read_only(),
            ),
        ]
    }

    async fn specs(&self) -> JidrResult<Vec<CommandSpec>> {
        self.client.call_as("commands.list", json!({})).await
    }

    async fn device(&self, selector: &str) -> JidrResult<Device> {
        let d: Value = self
            .client
            .call("device.details", json!({ "device": selector }))
            .await?;
        serde_json::from_value(d["device"].clone())
            .map_err(|e| JidrError::unexpected(e.to_string()))
    }

    async fn io(&self, selector: &str) -> JidrResult<DeviceIo> {
        let info: ConnectInfo = self
            .client
            .call_as("device.connect_info", json!({ "device": selector }))
            .await?;
        Ok(DeviceIo::new(info))
    }

    /// D5: destructive commands need `--allow-destructive` or an agent-owned device.
    async fn check_policy(&self, spec: &CommandSpec, params: &Value) -> JidrResult<()> {
        if !spec.destructive || self.opts.allow_destructive {
            return Ok(());
        }
        let selector = params.get("device").and_then(Value::as_str);
        if let Some(sel) = selector
            && self.device(sel).await?.agent_owned
        {
            return Ok(());
        }
        Err(JidrError::usage(format!(
            "{} is destructive and {} isn't agent-owned; ask the user to run it, to start `jidr mcp --allow-destructive`, or to mark the device agent-owned (`jidr device edit <name> --agent-owned true`)",
            spec.id,
            selector.unwrap_or("the target")
        ))
        .with_data(json!({ "policy": "D5", "command": spec.id })))
    }

    async fn command_run(&self, args: CommandRunArgs) -> JidrResult<Value> {
        if NOT_CALLABLE.contains(&args.command.as_str()) {
            return Err(JidrError::usage(format!(
                "{} can't be called through MCP",
                args.command
            )));
        }
        let specs = self.specs().await?;
        let spec = specs.iter().find(|s| s.id == args.command).ok_or_else(|| {
            JidrError::usage(format!(
                "unknown command {}; see command_list",
                args.command
            ))
        })?;
        let mut params = args.params.unwrap_or_else(|| json!({}));
        // Devices an agent creates are its own (D5), unless it says otherwise.
        if spec.id == "device.create"
            && let Some(o) = params.as_object_mut()
        {
            o.entry("agent_owned").or_insert(json!(true));
        }
        self.check_policy(spec, &params).await?;
        let v = self.client.call(&spec.id, params).await?;
        let Some(job) = jidr_client::pending_job(&v) else {
            return Ok(v);
        };
        let deadline =
            Instant::now() + Duration::from_secs(args.timeout_s.unwrap_or(DEFAULT_RUN_TIMEOUT_S));
        loop {
            let j = self
                .client
                .wait_job(job, Some(Duration::from_secs(5)))
                .await?;
            match j.state {
                jidr_core::JobState::Succeeded => return Ok(j.result.unwrap_or(Value::Null)),
                jidr_core::JobState::Failed => {
                    return Err(j
                        .error
                        .unwrap_or_else(|| JidrError::unexpected(format!("{} failed", spec.id))));
                }
                jidr_core::JobState::Cancelled => return Err(jidr_core::cancel::cancelled()),
                _ if Instant::now() >= deadline => {
                    return Ok(
                        json!({ "job": j, "pending": true, "note": "still running; check jobs_list or cancel with jobs_cancel" }),
                    );
                }
                _ => {}
            }
        }
    }

    async fn point(&self, io: &DeviceIo, x: f64, y: f64, coords: Coords) -> JidrResult<(u32, u32)> {
        match coords {
            Coords::Device => Ok((x.max(0.0).round() as u32, y.max(0.0).round() as u32)),
            Coords::Screenshot => Ok(device::to_device((x, y), io.screen_size().await?)),
        }
    }

    async fn dispatch(&self, name: &str, args: Option<JsonObject>) -> JidrResult<CallToolResult> {
        match name {
            "command_list" => {
                let a: CommandListArgs = parse(args)?;
                let specs: Vec<CommandSpec> = self
                    .specs()
                    .await?
                    .into_iter()
                    .filter(|s| a.prefix.as_deref().is_none_or(|p| s.id.starts_with(p)))
                    .collect();
                Ok(ok_json(json!({ "commands": specs })))
            }
            "command_run" => Ok(ok_json(self.command_run(parse(args)?).await?)),
            "jobs_list" => {
                let a: JobsListArgs = parse(args)?;
                Ok(ok_json(
                    json!({ "jobs": self.client.call("jobs.list", json!({ "all": a.all })).await? }),
                ))
            }
            "jobs_cancel" => {
                let a: JobArgs = parse(args)?;
                Ok(ok_json(
                    self.client
                        .call("jobs.cancel", json!({ "job": a.job }))
                        .await?,
                ))
            }
            "screenshot" => {
                let a: DeviceArgs = parse(args)?;
                let io = self.io(&a.device).await?;
                let (png, w, h) = io.screenshot().await?;
                let screen = io.screen_size().await.unwrap_or((w, h));
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
                let note = format!(
                    "Screenshot {w}×{h} of a {}×{} screen (×{:.3} to device pixels). tap/swipe use this image's coordinates by default.",
                    screen.0,
                    screen.1,
                    device::shot_scale(screen.0, screen.1)
                );
                let mut r = CallToolResult::success(vec![
                    ContentBlock::image(b64, "image/png"),
                    ContentBlock::text(note),
                ]);
                r.structured_content = Some(
                    json!({ "width": w, "height": h, "screen": { "width": screen.0, "height": screen.1 } }),
                );
                Ok(r)
            }
            "tap" => {
                let a: TapArgs = parse(args)?;
                let io = self.io(&a.device).await?;
                let (x, y) = self.point(&io, a.x, a.y, a.coords).await?;
                io.tap(x, y).await?;
                Ok(ok_json(json!({ "tapped": { "x": x, "y": y } })))
            }
            "swipe" => {
                let a: SwipeArgs = parse(args)?;
                let io = self.io(&a.device).await?;
                let from = self.point(&io, a.x1, a.y1, a.coords).await?;
                let to = self.point(&io, a.x2, a.y2, a.coords).await?;
                io.swipe(from, to, a.duration_ms.unwrap_or(300).clamp(10, 10_000))
                    .await?;
                Ok(ok_json(
                    json!({ "swiped": { "from": [from.0, from.1], "to": [to.0, to.1] } }),
                ))
            }
            "type_text" => {
                let a: TypeArgs = parse(args)?;
                let io = self.io(&a.device).await?;
                io.type_text(&a.text).await?;
                if a.enter {
                    io.press_key("Enter").await?;
                }
                Ok(ok_json(json!({ "typed": a.text.chars().count() })))
            }
            "press_key" => {
                let a: KeyArgs = parse(args)?;
                self.io(&a.device).await?.press_key(&a.key).await?;
                Ok(ok_json(json!({ "pressed": a.key })))
            }
            "ui_dump" => {
                let a: DeviceArgs = parse(args)?;
                let mut xml = self.io(&a.device).await?.ui_dump().await?;
                if xml.len() > UI_DUMP_LIMIT {
                    let cut = (0..=UI_DUMP_LIMIT)
                        .rev()
                        .find(|i| xml.is_char_boundary(*i))
                        .unwrap_or(0);
                    xml.truncate(cut);
                    xml.push_str("\n<!-- truncated -->");
                }
                Ok(CallToolResult::success(vec![ContentBlock::text(xml)]))
            }
            "logcat_tail" => {
                let a: LogcatArgs = parse(args)?;
                let v = self
                    .client
                    .call("logcat.dump", json!({ "device": a.device, "lines": a.lines.unwrap_or(200), "filter": a.filter }))
                    .await?;
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    v["text"].as_str().unwrap_or_default(),
                )]))
            }
            other => Err(JidrError::usage(format!("unknown tool {other}"))),
        }
    }
}

impl ServerHandler for JidrMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("jidr", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Jidr runs Android emulators (optionally rooted). Typical loop: command_run device.list / \
                 device.create {name, image: \"35\"} / device.start, then screenshot → tap/swipe/type_text → \
                 ui_dump to find elements. command_list shows every command. Devices you create are yours to delete.",
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(Self::tools())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        if !Self::tools().iter().any(|t| t.name == name) {
            return Err(McpError::invalid_params(
                format!("unknown tool {name}"),
                None,
            ));
        }
        let started = Instant::now();
        let result = match self.dispatch(&name, request.arguments).await {
            Ok(r) => r,
            Err(e) => tool_error(&e),
        };
        tracing::debug!(tool = %name, ms = started.elapsed().as_millis() as u64, "tool call");
        Ok(CallToolResponse::Complete(result))
    }
}
