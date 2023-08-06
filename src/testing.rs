use std::{
    net::SocketAddr,
    str::FromStr,
    time::{Duration, Instant},
};

use adns_client::DnsClient;
use adns_proto::{Question, Type};
use anyhow::{bail, Context, Result};
use log::warn;
use prometheus::{register_histogram_vec, register_int_gauge_vec, HistogramVec, IntGaugeVec};
use rand::{thread_rng, Rng};
use reqwest::redirect::Policy;
use surge_ping::{PingIdentifier, PingSequence};
use tokio::net::TcpStream;

use crate::config::{Mode, Target, CONFIG};

lazy_static::lazy_static! {
    static ref LATENCY: HistogramVec = register_histogram_vec!("reachr_latency_ms", "reachr_latency_ms", &["source", "host", "mode"], vec![1.0, 2.5, 5.0, 10.0, 20.0, 50.0, 75.0, 100.0, 200.0, 300.0, 500.0, 750.0, 1000.0, 1250.0, 1500.0, 2000.0]).unwrap();
    static ref REACHABILITY: IntGaugeVec = register_int_gauge_vec!("reachr_reachability", "reachr_reachability", &["source", "host", "mode"]).unwrap();

    static ref PING_CLIENT: surge_ping::Client = surge_ping::Client::new(&surge_ping::Config::default()).unwrap();
    static ref PINGV6_CLIENT: surge_ping::Client = surge_ping::Client::new(&surge_ping::Config { kind: surge_ping::ICMP::V6, ..Default::default() }).unwrap();
    static ref HTTP_CLIENT: reqwest::Client = reqwest::ClientBuilder::default().redirect(Policy::none()).build().unwrap();
}

impl Target {
    pub fn remove(&self) {
        let config = CONFIG.borrow();
        REACHABILITY
            .remove_label_values(&[&config.name, &self.host, &self.mode.name()])
            .ok();
        LATENCY
            .remove_label_values(&[&CONFIG.borrow().name, &self.host, &self.mode.name()])
            .ok();
    }

    pub async fn test(&self) {
        let result = match self.mode {
            Mode::Ping => self.test_ping().await,
            Mode::Tcp => self.test_tcp().await,
            Mode::Http => self.test_http().await,
            Mode::Https => self.test_http().await,
            Mode::Dns => self.test_dns().await,
        };
        match result {
            Ok(()) => {
                let config = CONFIG.borrow();
                REACHABILITY
                    .with_label_values(&[&config.name, &self.host, &self.mode.name()])
                    .set(1);
            }
            Err(e) => {
                warn!(
                    "failed to reach {}:{} via {}: {e:#}",
                    self.host,
                    self.port.unwrap_or(self.mode.port()),
                    self.mode.name()
                );
                let config = CONFIG.borrow();
                REACHABILITY
                    .with_label_values(&[&config.name, &self.host, &self.mode.name()])
                    .set(0);
            }
        }
    }

    async fn get_ip(&self) -> Result<Option<SocketAddr>> {
        Ok(
            tokio::net::lookup_host((&*self.host, self.port.unwrap_or(self.mode.port())))
                .await?
                .next(),
        )
    }

    async fn test_ping(&self) -> Result<()> {
        let Some(ip) = self.get_ip().await? else {
            bail!("no IP found (dns failure?)");
        };
        let timeout = Duration::from_secs(CONFIG.borrow().timeout);
        let id: u16 = thread_rng().gen();
        let client = if ip.is_ipv4() {
            &*PING_CLIENT
        } else {
            &*PINGV6_CLIENT
        };
        let (_, latency) = client
            .pinger(ip.ip(), PingIdentifier(id))
            .await
            .timeout(timeout)
            .ping(PingSequence(id), &[])
            .await?;
        LATENCY
            .with_label_values(&[&CONFIG.borrow().name, &self.host, &self.mode.name()])
            .observe(latency.as_secs_f64() * 1000.0);
        Ok(())
    }

    async fn test_tcp(&self) -> Result<()> {
        let Some(ip) = self.get_ip().await? else {
            bail!("no IP found (dns failure?)");
        };
        let timeout = Duration::from_secs(CONFIG.borrow().timeout);
        let start = Instant::now();
        tokio::time::timeout(timeout, TcpStream::connect(ip)).await??;
        LATENCY
            .with_label_values(&[&CONFIG.borrow().name, &self.host, &self.mode.name()])
            .observe(start.elapsed().as_secs_f64() * 1000.0);
        Ok(())
    }

    async fn test_http(&self) -> Result<()> {
        let timeout = Duration::from_secs(CONFIG.borrow().timeout);
        let start = Instant::now();
        let port = if self.port == None || self.port == Some(self.mode.port()) {
            "".to_string()
        } else {
            format!(":{}", self.port.unwrap())
        };
        let scheme = match self.mode {
            Mode::Http => "http",
            Mode::Https => "https",
            _ => unimplemented!(),
        };
        let response = HTTP_CLIENT
            .get(format!(
                "{}://{}{}{}",
                scheme,
                self.host,
                port,
                self.path.as_deref().unwrap_or("/")
            ))
            .timeout(timeout)
            .send()
            .await?;
        if response.status().as_u16() != self.status.unwrap_or(200) {
            bail!("unexpected status code: {}", response.status());
        }
        LATENCY
            .with_label_values(&[&CONFIG.borrow().name, &self.host, &self.mode.name()])
            .observe(start.elapsed().as_secs_f64() * 1000.0);
        Ok(())
    }

    async fn test_dns(&self) -> Result<()> {
        let Some(ip) = self.get_ip().await? else {
            bail!("no IP found (dns failure?)");
        };
        let mut client = DnsClient::new().await?;
        let timeout = Duration::from_secs(CONFIG.borrow().timeout);
        let start = Instant::now();
        let packet = tokio::time::timeout(
            timeout,
            client.query(
                ip,
                vec![Question {
                    name: self
                        .dns_name
                        .as_deref()
                        .context("missing dns_name for DNS type target")?
                        .parse()?,
                    type_: self
                        .r#type
                        .as_ref()
                        .map(|x| Type::from_str(x))
                        .transpose()?
                        .unwrap_or(Type::A),
                    class: adns_proto::Class::IN,
                }],
            ),
        )
        .await??;
        if packet.answers.is_empty() {
            bail!("no answers in DNS query");
        }
        LATENCY
            .with_label_values(&[&CONFIG.borrow().name, &self.host, &self.mode.name()])
            .observe(start.elapsed().as_secs_f64() * 1000.0);
        Ok(())
    }
}
