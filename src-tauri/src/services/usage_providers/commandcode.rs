//! Command Code usage: the 5-hour / weekly window meters plus the monthly
//! credit pool they pace.
//!
//! `/provider/v1` (the chat API the app talks to) has no usage route — a
//! `/provider/v1/usage` probe answers 404. Command Code's own `/usage` command
//! reads the same API key against the `/alpha/billing` routes, so that is what
//! this provider does: `credits` carries `windowLimits.{fiveHour,weekly}`
//! (`used` / `cap` / `resetAt` in ms) and the remaining monthly credits, and
//! `subscriptions` names the plan whose monthly pool those credits come from.
//!
//! `api.commandcode.ai` sits behind Cloudflare, which answers requests that
//! carry no User-Agent with `403 error code: 1010` — `send_usage_request`
//! therefore always sends one.

use super::{
    api_url, fetch_usage, parse_f64, parse_reset_time, QuotaPeriod, UsageProvider, UsageQuota,
    UsageResult,
};
use serde_json::Value;

const API_BASE: &str = "https://api.commandcode.ai";
const CREDITS_PATH: &str = "/alpha/billing/credits";
const SUBSCRIPTION_PATH: &str = "/alpha/billing/subscriptions";

pub struct CommandCodeProvider;

/// Monthly credit pool of a subscription plan, in USD. Unknown plans simply
/// lose the monthly row; the window meters are plan-independent.
fn plan_monthly_credits(plan_id: &str) -> Option<f64> {
    Some(match plan_id {
        "individual-go" => 10.0,
        "individual-goat" => 70.0,
        "individual-pro" => 80.0,
        "individual-max-10x" => 150.0,
        "individual-max-20x" => 300.0,
        "team-pro" => 40.0,
        _ => return None,
    })
}

fn used_percent(used: f64, cap: f64) -> Option<f64> {
    (cap > 0.0).then(|| (used / cap * 100.0).clamp(0.0, 100.0))
}

fn window_quota(credits: &Value, field: &str, period: QuotaPeriod) -> Option<UsageQuota> {
    let window = &credits["windowLimits"][field];
    let used = parse_f64(&window["used"])?;
    let cap = parse_f64(&window["cap"])?;
    UsageQuota::window(
        period,
        used_percent(used, cap)?,
        parse_reset_time(&window["resetAt"]),
    )
}

fn monthly_quota(credits: &Value, subscription: Option<&Value>) -> Option<UsageQuota> {
    let remaining = parse_f64(&credits["credits"]["monthlyCredits"])?;
    let plan = subscription
        .and_then(|body| body["data"]["planId"].as_str())
        .and_then(plan_monthly_credits)?;
    let used = (plan - remaining).max(0.0);
    UsageQuota::window(
        QuotaPeriod::Monthly,
        used_percent(used, plan)?,
        subscription.and_then(|body| parse_reset_time(&body["data"]["currentPeriodEnd"])),
    )
}

fn parse_quotas(credits: &Value, subscription: Option<&Value>) -> Vec<UsageQuota> {
    [
        window_quota(credits, "fiveHour", QuotaPeriod::FiveHour),
        window_quota(credits, "weekly", QuotaPeriod::Weekly),
        monthly_quota(credits, subscription),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[async_trait::async_trait]
impl UsageProvider for CommandCodeProvider {
    async fn query_usage(&self, api_key: &str, _base_url: &str) -> Result<UsageResult, String> {
        let authorization = format!("Bearer {api_key}");
        let credits = match fetch_usage(&format!("{API_BASE}{CREDITS_PATH}"), &authorization).await
        {
            Ok(body) => body,
            // `/alpha/billing` belongs to the web dashboard: it takes a Studio
            // session, not the API key that the Provider API accepts. A
            // provider key therefore gets 401 here while `/provider/v1` works.
            // Nothing is retryable, so report it as unsupported instead of a
            // failure the user keeps being asked to fix.
            Err(error) if crate::services::usage_providers::is_scope_error(&error) => {
                return Ok(UsageResult::unsupported(format!(
                    "{error} — the Command Code Provider API exposes no usage route"
                )));
            }
            Err(error) => return Ok(UsageResult::failure(error)),
        };
        // The subscription only names the monthly pool. A plan lookup that
        // fails (or an unknown plan) must not hide the window meters.
        let subscription = fetch_usage(&format!("{API_BASE}{SUBSCRIPTION_PATH}"), &authorization)
            .await
            .ok();
        Ok(UsageResult::from_quotas(parse_quotas(
            &credits,
            subscription.as_ref(),
        )))
    }

    fn can_handle(&self, base_url: &str) -> bool {
        api_url(base_url, &["api.commandcode.ai"])
            .is_some_and(|url| url.path() == "/provider" || url.path().starts_with("/provider/"))
    }

    fn name(&self) -> &'static str {
        "Command Code"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn credits() -> Value {
        json!({
            "credits": {"monthlyCredits": 68.1281322959, "purchasedCredits": 0, "freeCredits": 0},
            "windowLimits": {
                "limited": true,
                "fiveHour": {"used": 0.611605, "cap": 14, "exceeded": false, "resetAt": 1792943896871i64},
                "weekly": {"used": 1.8718677041, "cap": 35, "exceeded": false, "resetAt": 1793512688442i64}
            }
        })
    }

    fn subscription(plan: &str) -> Value {
        json!({"success": true, "data": {"planId": plan, "currentPeriodEnd": "2026-11-17T13:25:40.000Z"}})
    }

    #[test]
    fn a_key_without_a_billing_route_is_reported_as_unsupported() {
        use crate::services::usage_providers::is_scope_error;

        // What api.commandcode.ai actually answers for a Provider API key.
        let denied =
            "Usage authentication failed (HTTP 401): Invalid 'Authorization' header or token.";
        assert!(is_scope_error(denied));
        assert!(is_scope_error(
            "Usage access denied; check the key and subscription (HTTP 403)"
        ));
        // A flaky upstream stays a retryable failure.
        assert!(!is_scope_error("Usage API error (HTTP 502)"));
        assert!(!is_scope_error("Unable to reach the usage API"));

        let result = UsageResult::unsupported(denied);
        assert!(!result.success);
        assert!(result.unsupported);
        assert_eq!(result.error.as_deref(), Some(denied));
        assert!(result.data.is_none());
    }

    #[test]
    fn reports_window_meters_as_used_percentages() {
        let quotas = parse_quotas(&credits(), Some(&subscription("individual-goat")));
        assert_eq!(quotas.len(), 3);
        assert_eq!(quotas[0].period, Some(QuotaPeriod::FiveHour));
        // 0.611605 of a 14 cap is 4.4%, not 0.6 out of a percentage.
        assert!((quotas[0].percentage - 4.3686).abs() < 0.001);
        assert_eq!(quotas[0].reset_at, 1792943896871);
        assert_eq!(quotas[1].period, Some(QuotaPeriod::Weekly));
        assert!((quotas[1].percentage - 5.3482).abs() < 0.001);
        assert_eq!(quotas[2].period, Some(QuotaPeriod::Monthly));
        // $70 GOAT pool with $68.1281322959 left means $1.87 of 70 used.
        assert!((quotas[2].percentage - (1.8718677041 / 70.0 * 100.0)).abs() < 0.001);
        assert_eq!(quotas[2].reset_at, 1794921940000);
    }

    #[test]
    fn unknown_or_missing_plan_keeps_the_window_meters() {
        for subscription in [None, Some(subscription("individual-mystery"))] {
            let quotas = parse_quotas(&credits(), subscription.as_ref());
            assert_eq!(quotas.len(), 2);
            assert!(quotas
                .iter()
                .all(|quota| quota.period != Some(QuotaPeriod::Monthly)));
        }
    }

    #[test]
    fn malformed_windows_are_unknown_not_zero() {
        let quotas = parse_quotas(
            &json!({"windowLimits": {"fiveHour": {"used": 0, "cap": 0}, "weekly": {"used": "x", "cap": 35}}}),
            None,
        );
        assert!(quotas.is_empty());
    }

    #[test]
    fn separates_command_code_from_lookalike_urls() {
        for path in ["/provider", "/provider/v1", "/provider/v1/chat/completions"] {
            assert!(CommandCodeProvider.can_handle(&format!("https://api.commandcode.ai{path}")));
        }
        for url in [
            "https://api.commandcode.ai/alpha/billing/credits",
            "https://api.commandcode.ai.evil.test/provider/v1",
            "https://evil.test/api.commandcode.ai/provider/v1",
            "http://api.commandcode.ai/provider/v1",
        ] {
            assert!(!CommandCodeProvider.can_handle(url));
        }
    }
}
