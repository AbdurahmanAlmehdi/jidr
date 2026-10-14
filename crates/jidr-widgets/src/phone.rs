//! Phone widget: calls and SMS (gRPC `sendPhone`, `sendSms`) and the GSM
//! baseband: signal bars (`gsm signal-profile`, R3), data/voice state.

use jidr_backend::console::GsmLine;
use jidr_core::provider::{CallCtx, CommandSpec, parse_params};
use jidr_core::{JidrError, JidrResult};
use jidr_proto::control as pb;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::{grpc_err, target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallAction {
    /// An incoming call from `number`.
    Incoming,
    Accept,
    Reject,
    Busy,
    End,
    Hold,
    Unhold,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallParams {
    pub device: String,
    /// Phone number, digits with optional leading +.
    pub number: String,
    #[serde(default = "incoming")]
    pub action: CallAction,
}

fn incoming() -> CallAction {
    CallAction::Incoming
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmsParams {
    pub device: String,
    /// Sender number.
    pub from: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GsmState {
    Unregistered,
    Home,
    Roaming,
    Searching,
    Denied,
    Off,
    On,
}

impl GsmState {
    fn word(self) -> &'static str {
        match self {
            GsmState::Unregistered => "unregistered",
            GsmState::Home => "home",
            GsmState::Roaming => "roaming",
            GsmState::Searching => "searching",
            GsmState::Denied => "denied",
            GsmState::Off => "off",
            GsmState::On => "on",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GsmParams {
    pub device: String,
    /// Mobile signal bars, 0 (none) to 4 (great).
    #[serde(default)]
    pub signal: Option<u8>,
    #[serde(default)]
    pub data: Option<GsmState>,
    #[serde(default)]
    pub voice: Option<GsmState>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new::<CallParams>(
            "phone.call",
            "Simulate an incoming call or change its state",
        ),
        CommandSpec::new::<SmsParams>("phone.sms", "Deliver an SMS to the device"),
        CommandSpec::new::<GsmParams>(
            "phone.gsm",
            "Mobile signal bars (0–4) and data/voice registration",
        ),
    ]
}

/// Digits with an optional leading `+` (and spaces/dashes the user typed).
pub fn clean_number(n: &str) -> JidrResult<String> {
    let s: String = n
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')'))
        .collect();
    let digits = s.strip_prefix('+').unwrap_or(&s);
    if digits.is_empty() || digits.len() > 20 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(JidrError::usage(format!("not a phone number: {n:?}")));
    }
    Ok(s)
}

fn op(a: CallAction) -> pb::phone_call::Operation {
    use pb::phone_call::Operation as O;
    match a {
        CallAction::Incoming => O::InitCall,
        CallAction::Accept => O::AcceptCall,
        CallAction::Reject => O::RejectCallExplicit,
        CallAction::Busy => O::RejectCallBusy,
        CallAction::End => O::DisconnectCall,
        CallAction::Hold => O::PlaceCallOnHold,
        CallAction::Unhold => O::TakeCallOffHold,
    }
}

fn check_response(what: &str, r: &pb::PhoneResponse) -> JidrResult<()> {
    if r.response == pb::phone_response::Response::Ok as i32 {
        Ok(())
    } else {
        Err(JidrError::usage(format!(
            "{what} refused by the modem ({})",
            r.response
        )))
    }
}

pub async fn call(ctx: &CallCtx, method: &str, params: Value) -> JidrResult<Value> {
    match method {
        "phone.call" => {
            let p: CallParams = parse_params(params)?;
            let number = clean_number(&p.number)?;
            let t = target(ctx, &p.device).await?;
            let r = t
                .grpc()
                .await?
                .send_phone(pb::PhoneCall {
                    operation: op(p.action) as i32,
                    number: number.clone(),
                })
                .await
                .map_err(grpc_err("sendPhone"))?;
            check_response("call", r.get_ref())?;
            Ok(json!({ "number": number, "action": p.action }))
        }
        "phone.sms" => {
            let p: SmsParams = parse_params(params)?;
            let from = clean_number(&p.from)?;
            if p.text.is_empty() {
                return Err(JidrError::usage("the SMS text is empty"));
            }
            let t = target(ctx, &p.device).await?;
            let r = t
                .grpc()
                .await?
                .send_sms(pb::SmsMessage {
                    src_address: from.clone(),
                    text: p.text,
                })
                .await
                .map_err(grpc_err("sendSms"))?;
            check_response("SMS", r.get_ref())?;
            Ok(json!({ "from": from }))
        }
        "phone.gsm" => {
            let p: GsmParams = parse_params(params)?;
            if p.signal.is_some_and(|s| s > 4) {
                return Err(JidrError::usage("signal is 0–4 bars"));
            }
            let t = target(ctx, &p.device).await?;
            let mut c = t.console().await?;
            if let Some(s) = p.signal {
                c.command(&format!("gsm signal-profile {s}")).await?;
            }
            if let Some(d) = p.data {
                c.gsm_state(GsmLine::Data, d.word()).await?;
            }
            if let Some(v) = p.voice {
                c.gsm_state(GsmLine::Voice, v.word()).await?;
            }
            Ok(json!({ "signal": p.signal, "data": p.data, "voice": p.voice }))
        }
        _ => Err(JidrError::usage(format!("unknown method {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_cleaned_and_checked() {
        assert_eq!(clean_number("+218 91-234 5678").unwrap(), "+218912345678");
        assert!(clean_number("abc").is_err());
        assert!(clean_number("+").is_err());
        assert!(clean_number("12; reboot").is_err());
    }
}
