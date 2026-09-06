//! The webhook sink: the [`Outgoing`] as a JSON body, 2xx is delivered.

use std::collections::BTreeMap;

use super::{Delivery, Outgoing, SinkError};
use crate::config::SinkConfig;

#[derive(Debug)]
pub struct Webhook {
    client: reqwest::Client,
    url: String,
    method: reqwest::Method,
    headers: BTreeMap<String, String>,
}

impl Webhook {
    pub fn build(cfg: &SinkConfig, client: &reqwest::Client) -> Result<Webhook, String> {
        let SinkConfig::Webhook {
            url,
            method,
            headers,
            ..
        } = cfg
        else {
            return Err("not a webhook config".into());
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| format!("{method:?} is not an HTTP method"))?;
        let headers = headers
            .iter()
            .map(|(k, v)| v.resolve().map(|v| (k.clone(), v)))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        Ok(Webhook {
            client: client.clone(),
            url: url.clone(),
            method,
            headers,
        })
    }

    pub async fn deliver(&self, o: &Outgoing) -> Result<Delivery, SinkError> {
        let mut req = self.client.request(self.method.clone(), &self.url);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let resp = req
            .json(o)
            .send()
            .await
            .map_err(|e| SinkError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            Ok(Delivery {
                detail: format!("HTTP {status}"),
            })
        } else {
            Err(SinkError::Http {
                status: status.as_u16(),
                body: super::http::body_head(resp, 256).await,
            })
        }
    }

    pub fn probe(&self) -> Result<String, SinkError> {
        reqwest::Url::parse(&self.url)
            .map(|u| {
                format!(
                    "{} {u} parses; a webhook has no side-effect-free probe",
                    self.method
                )
            })
            .map_err(|e| SinkError::Probe(e.to_string()))
    }
}
