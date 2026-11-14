//! The helper's request loop, independent of the socket so it runs (and is
//! tested) on any host. The binary wires it to the abstract socket.

use std::io::{BufRead, BufReader, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::identity;
use crate::netem::{self, Run};
use crate::rpc::{
    AndroidIdSet, Error, Method, Ping, Pong, Request, Response, ShapingGet, ShapingSet, Shutdown,
};

/// Answers requests: `ping` with `pong`, the rest through `run`.
pub struct Server<R: Run> {
    pub pong: Pong,
    pub run: R,
}

/// What to do after a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Continue,
    /// `shutdown` was answered; the process should exit.
    Exit,
}

fn params<T: DeserializeOwned>(params: Value) -> Result<T, Error> {
    let params = if params.is_null() {
        Value::Object(Default::default())
    } else {
        params
    };
    serde_json::from_value(params).map_err(|e| Error::usage(format!("params: {e}")))
}

fn result<T: Serialize>(v: Result<T, Error>) -> Result<Value, Error> {
    v.and_then(|v| {
        serde_json::to_value(v).map_err(|e| Error::failed(format!("serializing result: {e}")))
    })
}

impl<R: Run> Server<R> {
    pub fn new(pong: Pong, run: R) -> Self {
        Self { pong, run }
    }

    /// Runs one request.
    pub fn handle(&self, req: Request) -> (Response, Next) {
        let out = match req.method.as_str() {
            Ping::NAME => params::<Ping>(req.params).and_then(|_| result(Ok(&self.pong))),
            Shutdown::NAME => {
                return (Response::ok(req.id, Value::Null), Next::Exit);
            }
            ShapingGet::NAME => params(req.params).and_then(|p| result(netem::get(&self.run, &p))),
            ShapingSet::NAME => params(req.params).and_then(|p| result(netem::set(&self.run, &p))),
            AndroidIdSet::NAME => {
                params(req.params).and_then(|p| result(identity::set(&self.run, &p)))
            }
            other => Err(Error::usage(format!("unknown method {other}"))),
        };
        let resp = match out {
            Ok(v) => Response::ok(req.id, v),
            Err(e) => Response::err(req.id, e),
        };
        (resp, Next::Continue)
    }

    /// Answers each line of `stream` until it closes or asks to shut down.
    pub fn serve<S: std::io::Read + Write>(&self, stream: S) -> std::io::Result<Next> {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(Next::Continue);
            }
            if line.trim().is_empty() {
                continue;
            }
            let (resp, next) = match serde_json::from_str::<Request>(&line) {
                Ok(req) => self.handle(req),
                Err(e) => (
                    Response::err(0, Error::usage(format!("not a request: {e}"))),
                    Next::Continue,
                ),
            };
            let mut out = serde_json::to_vec(&resp)?;
            out.push(b'\n');
            let w = reader.get_mut();
            w.write_all(&out)?;
            w.flush()?;
            if next == Next::Exit {
                return Ok(Next::Exit);
            }
        }
    }
}
