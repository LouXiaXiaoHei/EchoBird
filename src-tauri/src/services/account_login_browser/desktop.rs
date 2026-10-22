//! Manus and Cue return desktop sessions by URI, not by the web login cookie.
use rand::RngCore;
use std::sync::{Arc, Mutex};
use url::Url;

#[derive(Clone)]
pub(super) struct DesktopLogin {
    scheme: &'static str,
    nonce: String,
    token: Arc<Mutex<Option<Result<String, String>>>>,
}

impl DesktopLogin {
    pub fn new(scheme: &'static str) -> Self {
        let mut nonce = [0; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        Self {
            scheme,
            nonce: hex::encode(nonce),
            token: Default::default(),
        }
    }

    pub fn url(&self) -> Url {
        let mut redirect = Url::parse("https://manus.im/login-success").unwrap();
        redirect.query_pairs_mut().append_pair("nonce", &self.nonce);
        if self.scheme == "cue" {
            redirect
                .query_pairs_mut()
                .append_pair("desktopAgents", "1")
                .append_pair("agentsScheme", "cue");
        }
        let mut url = Url::parse("https://manus.im/login").unwrap();
        url.query_pairs_mut()
            .append_pair("from", "desktop")
            .append_pair("type", "signIn")
            .append_pair("nonce", &self.nonce)
            .append_pair("redirectUrl", redirect.as_str());
        if self.scheme == "cue" {
            url.query_pairs_mut().append_pair("desktopAgents", "1");
        }
        url
    }

    pub fn capture(&self, url: &Url) {
        let (host, path) = if self.scheme == "cue" {
            ("oauth", "/desktop-callback")
        } else {
            ("login", "")
        };
        if url.as_str().len() > 65_536
            || url.scheme() != self.scheme
            || url.host_str() != Some(host)
            || url.path() != path
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
        {
            return;
        }
        let values = url.query_pairs().collect::<Vec<_>>();
        let one = |key| {
            let mut matches = values.iter().filter(|(name, _)| name == key);
            let value = matches.next()?.1.as_ref();
            matches.next().is_none().then_some(value)
        };
        if one("nonce") != Some(self.nonce.as_str()) {
            return;
        }
        let token = one("token").filter(|token| {
            !token.is_empty() && token.len() <= 32_768 && !token.contains(['\r', '\n', ';'])
        });
        let error = one("error");
        let result = match (token, error) {
            (Some(token), None) if !values.iter().any(|(key, _)| key == "error") => {
                Ok(token.to_owned())
            }
            (None, Some(_)) if !values.iter().any(|(key, _)| key == "token") => {
                Err("accountError.loginRequired".into())
            }
            _ => return,
        };
        if let Ok(mut current) = self.token.lock() {
            if current.is_none() {
                *current = Some(result);
            }
        }
    }

    pub fn token(&self) -> Result<Option<String>, String> {
        self.token
            .lock()
            .map_err(|_| "accountError.busy")?
            .clone()
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_login_params_keep_each_products_official_return_flow() {
        for scheme in ["manus", "cue"] {
            let login = DesktopLogin::new(scheme);
            let url = login.url();
            let query = url
                .query_pairs()
                .collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(query["from"], "desktop");
            assert_eq!(query["type"], "signIn");
            assert_eq!(query["nonce"], login.nonce);
            assert_eq!(login.nonce.len(), 64);
            let redirect = Url::parse(&query["redirectUrl"]).unwrap();
            assert_eq!(redirect.host_str(), Some("manus.im"));
            assert_eq!(redirect.path(), "/login-success");
            assert!(redirect
                .query_pairs()
                .any(|(k, v)| k == "nonce" && v == login.nonce));
            assert_eq!(
                query.get("desktopAgents").map(|v| v.as_ref()),
                (scheme == "cue").then_some("1")
            );
        }
        assert_ne!(
            DesktopLogin::new("cue").nonce,
            DesktopLogin::new("cue").nonce
        );
    }

    #[test]
    fn official_callbacks_capture_once_without_any_desktop_cookie() {
        for (scheme, route) in [
            ("manus", "manus://login"),
            ("cue", "cue://oauth/desktop-callback"),
        ] {
            let login = DesktopLogin::new(scheme);
            assert!(login.token().unwrap().is_none());
            let mut url = Url::parse(route).unwrap();
            url.query_pairs_mut()
                .append_pair("nonce", &login.nonce)
                .append_pair("token", "fixture-session");
            login.capture(&url);
            assert_eq!(login.token().unwrap().as_deref(), Some("fixture-session"));
            url.query_pairs_mut()
                .clear()
                .append_pair("nonce", &login.nonce)
                .append_pair("token", "replacement");
            login.capture(&url);
            assert_eq!(login.token().unwrap().as_deref(), Some("fixture-session"));
        }
    }

    #[test]
    fn spoofed_duplicated_and_cross_attempt_callbacks_cannot_supply_a_session() {
        let login = DesktopLogin::new("cue");
        for route in [
            "https://oauth/desktop-callback",
            "manus://oauth/desktop-callback",
            "cue://other/desktop-callback",
            "cue://oauth/other",
            "cue://user@oauth/desktop-callback",
            "cue://oauth:443/desktop-callback",
            "cue://oauth/desktop-callback#fragment",
        ] {
            let mut url = Url::parse(route).unwrap();
            url.query_pairs_mut()
                .append_pair("nonce", &login.nonce)
                .append_pair("token", "fixture");
            login.capture(&url);
            assert!(login.token().unwrap().is_none());
        }
        for query in [
            format!("nonce={}&token=one&token=two", login.nonce),
            format!("nonce={0}&nonce={0}&token=fixture", login.nonce),
            format!("nonce={}&token=fixture&error=denied", login.nonce),
            format!("nonce={}&token=bad%0D%0Avalue", login.nonce),
            format!("nonce={}&token=fixture", DesktopLogin::new("cue").nonce),
        ] {
            login.capture(&Url::parse(&format!("cue://oauth/desktop-callback?{query}")).unwrap());
            assert!(login.token().unwrap().is_none());
        }
        let denied = Url::parse(&format!(
            "cue://oauth/desktop-callback?nonce={}&error=denied",
            login.nonce
        ))
        .unwrap();
        login.capture(&denied);
        assert_eq!(login.token().unwrap_err(), "accountError.loginRequired");
    }
}
