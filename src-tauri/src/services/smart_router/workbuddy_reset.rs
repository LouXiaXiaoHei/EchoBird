//! WorkBuddy's request refusals and explicit per-model quota reset.
use super::{AccountProvider, Candidate, FailureClass, StatusCode};
use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use serde_json::Value;

pub(super) struct WorkBuddyFailure {
    pub failure: FailureClass,
    pub reset_at_ms: Option<u64>,
}

pub(super) fn classify(
    candidate: &Candidate,
    status: StatusCode,
    body: &[u8],
    now: u64,
) -> Option<WorkBuddyFailure> {
    if !candidate.account_source.as_ref().is_some_and(|source| {
        matches!(
            source.provider,
            AccountProvider::WorkBuddy | AccountProvider::WorkBuddyAi
        )
    }) || !(status.is_client_error() || status.is_server_error())
    {
        return None;
    }
    let value: Value = serde_json::from_slice(body).ok()?;
    let error = value
        .pointer("/response/error")
        .filter(|v| v.is_object())
        .or_else(|| value.get("error").filter(|v| v.is_object()))
        .unwrap_or(&value);
    if error["code"] == 11128 || error["code"] == "11128" {
        return Some(WorkBuddyFailure {
            failure: FailureClass::RequestIncompatible,
            reset_at_ms: None,
        });
    }
    if error["code"] != 6004 && error["code"] != "6004" {
        return None;
    }
    let message = ["message", "msg", "error_msg"]
        .iter()
        .find_map(|key| error[*key].as_str())
        .unwrap_or_default();
    Some(WorkBuddyFailure {
        failure: FailureClass::RateLimit,
        reset_at_ms: written_reset(message, now),
    })
}

fn written_reset(message: &str, now: u64) -> Option<u64> {
    for zone in ["UTC+08:00", "UTC+8"] {
        for (offset, _) in message.match_indices(zone) {
            if message[offset + zone.len()..].starts_with(|c: char| c.is_ascii_digit() || c == ':')
            {
                continue;
            }
            let before = message[..offset]
                .trim_end()
                .trim_end_matches('(')
                .trim_end();
            let Some(date) = before.get(before.len().saturating_sub(19)..) else {
                continue;
            };
            let Ok(date) =
                NaiveDateTime::parse_from_str(&date.replace('T', " "), "%Y-%m-%d %H:%M:%S")
            else {
                continue;
            };
            let reset = FixedOffset::east_opt(8 * 3600)?
                .from_local_datetime(&date)
                .single()?
                .timestamp_millis();
            if let Ok(reset) = u64::try_from(reset) {
                if reset > now && reset - now <= 7 * 24 * 60 * 60 * 1000 {
                    return Some(reset);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_time_has_an_explicit_beijing_offset_and_bounded_future_deadline() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T00:00:00Z")
            .unwrap()
            .timestamp_millis() as u64;
        for message in [
            "于 2026-10-22 09:00:00 (UTC+8) 可用",
            "will reset at 2026-10-22 09:00:00 UTC+08:00, alternatively use another model",
        ] {
            assert_eq!(written_reset(message, now), Some(now + 3_600_000));
        }
        for message in [
            "2026-10-22 09:00:00",
            "2026-10-22 09:00:00 UTC+0",
            "2026-10-22 09:00:00 UTC+80",
            "2026-02-30 09:00:00 UTC+8",
            "2026-10-22 07:59:59 UTC+8",
            "2026-11-22 09:00:00 UTC+8",
            "2026-10-22 09:00 UTC+8",
            "2026-10-22 09:00:00 UTC+8:30",
        ] {
            assert_eq!(written_reset(message, now), None, "{message}");
        }
    }

    #[test]
    fn only_workbuddy_quota_codes_supply_a_reset_and_missing_dates_keep_normal_cooldown() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T00:00:00Z")
            .unwrap()
            .timestamp_millis() as u64;
        let mut candidate = Candidate {
            attempt: None,
            internal_id: "fixture".into(),
            fingerprint: "fixture".into(),
            model_id: "model".into(),
            base_url: "https://fixture.invalid".into(),
            api_key: String::new(),
            account_source: Some(crate::models::model::AccountSource {
                provider: AccountProvider::WorkBuddy,
                account_id: "fixture-account".into(),
            }),
        };
        let body = serde_json::json!({"error":{"code":"6004","message":"于 2026-10-22 09:00:00 UTC+8 可用"}}).to_string();
        assert_eq!(
            classify(&candidate, StatusCode::BAD_REQUEST, body.as_bytes(), now)
                .unwrap()
                .reset_at_ms,
            Some(now + 3_600_000)
        );
        assert!(classify(&candidate, StatusCode::OK, body.as_bytes(), now).is_none());
        let unrecognized = serde_json::json!({"code":500,"msg":"2026-10-22 09:00:00 UTC+8"});
        assert!(classify(
            &candidate,
            StatusCode::BAD_REQUEST,
            unrecognized.to_string().as_bytes(),
            now
        )
        .is_none());
        let refused = serde_json::json!({"code":11128,"msg":"Illegal API invocation from an unapproved channel"}).to_string();
        let refused =
            classify(&candidate, StatusCode::BAD_REQUEST, refused.as_bytes(), now).unwrap();
        assert_eq!(refused.failure, FailureClass::RequestIncompatible);
        assert_eq!(refused.reset_at_ms, None);
        let unknown = serde_json::json!({"code":6004,"msg":"No reset time supplied"}).to_string();
        assert!(
            classify(&candidate, StatusCode::BAD_REQUEST, unknown.as_bytes(), now)
                .unwrap()
                .reset_at_ms
                .is_none()
        );
        candidate.account_source.as_mut().unwrap().provider = AccountProvider::Traecn;
        assert!(classify(&candidate, StatusCode::BAD_REQUEST, body.as_bytes(), now).is_none());
        candidate.account_source = None;
        assert!(classify(&candidate, StatusCode::BAD_REQUEST, body.as_bytes(), now).is_none());
    }
}
