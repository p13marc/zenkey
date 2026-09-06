//! The SMTP sink: one plain-text mail per notification through a relay.

use std::time::Duration;

use lettre::message::{Mailbox, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport as _, Message, Tokio1Executor};

use super::{Delivery, Outgoing, SinkError};
use crate::config::{SinkConfig, SmtpTls};

#[derive(Debug)]
pub struct Smtp {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Vec<Mailbox>,
    subject_prefix: String,
}

impl Smtp {
    pub fn build(cfg: &SinkConfig) -> Result<Smtp, String> {
        let SinkConfig::Smtp {
            host,
            port,
            tls,
            username,
            password,
            from,
            to,
            subject_prefix,
            timeout_s,
        } = cfg
        else {
            return Err("not an smtp config".into());
        };
        let builder = match tls {
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host),
            SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host),
            SmtpTls::Dangerous => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                host,
            )),
        }
        .map_err(|e| e.to_string())?;
        let mut builder = builder
            .port(*port)
            .timeout(Some(Duration::from_secs_f64(timeout_s.unwrap_or(10.0))));
        if let (Some(u), Some(p)) = (username, password) {
            let secret = p.resolve().map_err(|e| e.to_string())?;
            builder = builder.credentials(Credentials::new(u.clone(), secret));
        }
        let from: Mailbox = from.parse().map_err(|e| format!("from {from:?}: {e}"))?;
        let to = to
            .iter()
            .map(|t| t.parse::<Mailbox>().map_err(|e| format!("to {t:?}: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Smtp {
            transport: builder.build(),
            from,
            to,
            subject_prefix: subject_prefix.clone(),
        })
    }

    pub async fn deliver(&self, o: &Outgoing) -> Result<Delivery, SinkError> {
        let n = &o.notification;
        let mut builder = Message::builder()
            .from(self.from.clone())
            .subject(format!("{} {}", self.subject_prefix, n.title))
            .header(ContentType::TEXT_PLAIN);
        for rcpt in &self.to {
            builder = builder.to(rcpt.clone());
        }
        let mail = builder
            .body(n.message.clone())
            .map_err(|e| SinkError::Smtp(e.to_string()))?;
        let resp = self
            .transport
            .send(mail)
            .await
            .map_err(|e| SinkError::Smtp(e.to_string()))?;
        Ok(Delivery {
            detail: format!("SMTP {}", resp.code()),
        })
    }

    /// `EHLO` and, with credentials, `AUTH` — nothing sent.
    pub async fn probe(&self) -> Result<String, SinkError> {
        match self.transport.test_connection().await {
            Ok(true) => Ok("relay answered".into()),
            Ok(false) => Err(SinkError::Probe("relay did not answer".into())),
            Err(e) => Err(SinkError::Probe(e.to_string())),
        }
    }
}
