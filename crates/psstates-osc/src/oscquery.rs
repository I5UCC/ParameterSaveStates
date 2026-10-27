//! OSCQuery JSON model and HTTP client.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use psstates_core::{ParamType, ParamValue};
use serde::{Deserialize, Serialize};

pub const ACCESS_NONE: i64 = 0;
pub const ACCESS_READ: i64 = 1;
pub const ACCESS_WRITE: i64 = 2;
pub const ACCESS_READ_WRITE: i64 = 3;

/// One node of an OSCQuery namespace tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OscQueryNode {
    #[serde(rename = "FULL_PATH", default)]
    pub full_path: String,
    #[serde(rename = "ACCESS", default, skip_serializing_if = "Option::is_none")]
    pub access: Option<i64>,
    #[serde(rename = "TYPE", default, skip_serializing_if = "Option::is_none")]
    pub osc_type: Option<String>,
    #[serde(rename = "VALUE", default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<serde_json::Value>>,
    #[serde(
        rename = "DESCRIPTION",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub description: Option<String>,
    #[serde(rename = "CONTENTS", default, skip_serializing_if = "Option::is_none")]
    pub contents: Option<BTreeMap<String, OscQueryNode>>,
}

impl OscQueryNode {
    pub fn container(full_path: impl Into<String>) -> Self {
        Self {
            full_path: full_path.into(),
            access: Some(ACCESS_NONE),
            contents: Some(BTreeMap::new()),
            ..Default::default()
        }
    }

    /// Finds a node by absolute path, e.g. `/avatar/change`.
    pub fn find(&self, path: &str) -> Option<&OscQueryNode> {
        let mut node = self;
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            node = node.contents.as_ref()?.get(segment)?;
        }
        Some(node)
    }

    /// Inserts a leaf at an absolute path, creating containers as needed.
    pub fn insert(&mut self, path: &str, leaf: OscQueryNode) {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut node = self;
        let mut current = String::new();
        for (i, segment) in segments.iter().enumerate() {
            current.push('/');
            current.push_str(segment);
            let contents = node.contents.get_or_insert_with(BTreeMap::new);
            node = if i + 1 == segments.len() {
                contents.insert((*segment).to_string(), leaf.clone());
                contents.get_mut(*segment).unwrap()
            } else {
                contents
                    .entry((*segment).to_string())
                    .or_insert_with(|| OscQueryNode::container(current.clone()))
            };
        }
    }

    /// First value as a string, if any.
    pub fn first_string(&self) -> Option<&str> {
        self.value.as_ref()?.first()?.as_str()
    }

    /// The typed value of a leaf node.
    pub fn param_value(&self) -> Option<ParamValue> {
        let ty = ParamType::from_osc_tag(self.osc_type.as_deref()?)?;
        let v = self.value.as_ref()?.first()?;
        match ty {
            ParamType::Float => v
                .as_f64()
                .map(|f| f as f32)
                .filter(|f| f.is_finite())
                .map(ParamValue::Float),
            ParamType::Int => v
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .map(ParamValue::Int),
            ParamType::Bool => v.as_bool().map(ParamValue::Bool),
        }
    }

    /// Collects all typed leaves below (and including) this node.
    pub fn collect_parameters(&self, out: &mut BTreeMap<String, ParamValue>) {
        if let Some(v) = self.param_value() {
            if !self.full_path.is_empty() {
                out.insert(self.full_path.clone(), v);
            }
        }
        if let Some(contents) = &self.contents {
            for child in contents.values() {
                child.collect_parameters(out);
            }
        }
    }
}

/// OSCQuery `HOST_INFO` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HostInfo {
    #[serde(rename = "NAME", default)]
    pub name: String,
    #[serde(rename = "EXTENSIONS", default)]
    pub extensions: BTreeMap<String, bool>,
    #[serde(rename = "OSC_IP", default)]
    pub osc_ip: String,
    #[serde(rename = "OSC_PORT", default)]
    pub osc_port: u16,
    #[serde(rename = "OSC_TRANSPORT", default)]
    pub osc_transport: String,
}

/// HTTP client for a remote OSCQuery service (VRChat).
#[derive(Debug, Clone)]
pub struct OscQueryClient {
    http: reqwest::Client,
    addr: SocketAddr,
}

impl OscQueryClient {
    pub fn new(addr: SocketAddr) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .no_proxy()
            .build()
            .expect("reqwest client");
        Self { http, addr }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub async fn host_info(&self) -> Result<HostInfo> {
        let resp = self
            .http
            .get(self.url("/?HOST_INFO"))
            .send()
            .await
            .context("OSCQuery HOST_INFO request failed")?
            .error_for_status()?;
        resp.json().await.context("Invalid HOST_INFO response")
    }

    /// Fetches a node (and its subtree). Returns `None` on HTTP 404.
    pub async fn node(&self, path: &str) -> Result<Option<OscQueryNode>> {
        let resp = self
            .http
            .get(self.url(path))
            .send()
            .await
            .with_context(|| format!("OSCQuery request for '{path}' failed"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = resp.error_for_status()?;
        Ok(Some(resp.json().await.with_context(|| {
            format!("Invalid OSCQuery response for '{path}'")
        })?))
    }

    /// Current avatar id (`/avatar/change` value).
    pub async fn current_avatar_id(&self) -> Result<Option<String>> {
        Ok(self
            .node("/avatar/change")
            .await?
            .and_then(|n| n.first_string().map(str::to_string))
            .filter(|s| !s.is_empty()))
    }

    /// Current avatar display name (`/avatar/name` value), if VRChat exposes it.
    pub async fn current_avatar_name(&self) -> Result<Option<String>> {
        Ok(self
            .node("/avatar/name")
            .await?
            .and_then(|n| n.first_string().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty()))
    }

    /// All current avatar parameter values from `/avatar/parameters`.
    pub async fn avatar_parameters(&self) -> Result<BTreeMap<String, ParamValue>> {
        let mut out = BTreeMap::new();
        if let Some(node) = self.node("/avatar/parameters").await? {
            node.collect_parameters(&mut out);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VRC_TREE: &str = r#"{
      "FULL_PATH": "/avatar/parameters", "ACCESS": 0,
      "CONTENTS": {
        "Hue": { "FULL_PATH": "/avatar/parameters/Hue", "ACCESS": 3, "TYPE": "f", "VALUE": [0.25] },
        "Mode": { "FULL_PATH": "/avatar/parameters/Mode", "ACCESS": 3, "TYPE": "i", "VALUE": [2] },
        "On": { "FULL_PATH": "/avatar/parameters/On", "ACCESS": 3, "TYPE": "T", "VALUE": [true] },
        "FT": { "FULL_PATH": "/avatar/parameters/FT", "ACCESS": 0, "CONTENTS": {
          "Eye": { "FULL_PATH": "/avatar/parameters/FT/Eye", "ACCESS": 3, "TYPE": "f", "VALUE": [1] }
        } },
        "Str": { "FULL_PATH": "/avatar/parameters/Str", "ACCESS": 1, "TYPE": "s", "VALUE": ["x"] },
        "Empty": { "FULL_PATH": "/avatar/parameters/Empty", "ACCESS": 3, "TYPE": "f" }
      }
    }"#;

    #[test]
    fn collects_parameters() {
        let node: OscQueryNode = serde_json::from_str(VRC_TREE).unwrap();
        let mut out = BTreeMap::new();
        node.collect_parameters(&mut out);
        assert_eq!(out.len(), 4);
        assert_eq!(out["/avatar/parameters/Hue"], ParamValue::Float(0.25));
        assert_eq!(out["/avatar/parameters/Mode"], ParamValue::Int(2));
        assert_eq!(out["/avatar/parameters/On"], ParamValue::Bool(true));
        assert_eq!(out["/avatar/parameters/FT/Eye"], ParamValue::Float(1.0));
        assert!(node.find("/FT/Eye").is_some());
    }

    #[test]
    fn insert_builds_containers() {
        let mut root = OscQueryNode::container("/");
        root.insert(
            "/avatar/change",
            OscQueryNode {
                full_path: "/avatar/change".into(),
                access: Some(ACCESS_WRITE),
                osc_type: Some("s".into()),
                ..Default::default()
            },
        );
        assert_eq!(root.find("/avatar").unwrap().full_path, "/avatar");
        assert_eq!(
            root.find("/avatar/change").unwrap().osc_type.as_deref(),
            Some("s")
        );
    }
}
