//! An rmcp client talking to an in-process `JidrMcp` over a duplex pipe.
#![allow(dead_code)]

use std::time::Instant;

use jidr_mcp::{JidrMcp, McpOptions};
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

pub struct Mcp {
    client: RunningService<RoleClient, ()>,
    _server: tokio::task::JoinHandle<()>,
    /// (tool, milliseconds) of every call, for the latency report.
    pub timings: std::sync::Mutex<Vec<(String, u128)>>,
}

impl Mcp {
    pub async fn connect(jidr: jidr_client::Client, opts: McpOptions) -> Self {
        let (server_io, client_io) = tokio::io::duplex(8 << 20);
        let server = tokio::spawn(async move {
            if let Ok(running) = JidrMcp::new(jidr, opts).serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let client = ().serve(client_io).await.unwrap();
        Self {
            client,
            _server: server,
            timings: Default::default(),
        }
    }

    pub async fn tools(&self) -> Vec<Tool> {
        self.client.list_all_tools().await.unwrap()
    }

    /// Calls a tool; returns the result (errors are results with is_error).
    pub async fn call(&self, name: &str, args: Value) -> CallToolResult {
        let params = CallToolRequestParams::new(name.to_string())
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        let t = Instant::now();
        let r = self.client.call_tool(params).await.unwrap();
        self.timings
            .lock()
            .unwrap()
            .push((name.to_string(), t.elapsed().as_millis()));
        r
    }

    /// A successful call's structured content.
    pub async fn ok(&self, name: &str, args: Value) -> Value {
        let r = self.call(name, args.clone()).await;
        assert_ne!(r.is_error, Some(true), "{name} {args} failed: {}", text(&r));
        r.structured_content.clone().unwrap_or(Value::Null)
    }

    /// A failed call's structured error `{code, message, data}`.
    pub async fn err(&self, name: &str, args: Value) -> Value {
        let r = self.call(name, args.clone()).await;
        assert_eq!(
            r.is_error,
            Some(true),
            "{name} {args} unexpectedly succeeded: {}",
            text(&r)
        );
        r.structured_content.clone().unwrap_or(Value::Null)
    }

    pub fn report(&self) {
        let t = self.timings.lock().unwrap();
        let mut by: std::collections::BTreeMap<&str, Vec<u128>> = Default::default();
        for (n, ms) in t.iter() {
            by.entry(n).or_default().push(*ms);
        }
        println!("MCP tool latency (ms), not gated:");
        for (n, mut v) in by {
            v.sort_unstable();
            println!(
                "  {n:<14} calls {:>3}  median {:>6}  max {:>6}",
                v.len(),
                v[v.len() / 2],
                v[v.len() - 1]
            );
        }
    }
}

/// All text content of a result.
pub fn text(r: &CallToolResult) -> String {
    r.content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
